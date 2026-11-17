//! Human authentication: passkeys only.
//!
//! First run: the server prints a one-time setup code. Whoever has it (you, reading your own
//! server logs) can enroll the first passkey. After that, new passkeys can only be added from a
//! signed-in session. Sessions are opaque random cookies; only their SHA-256 is stored.

use axum::{
    Json,
    extract::{FromRequestParts, Request, State},
    http::{HeaderMap, HeaderValue, Method, header, request::Parts},
    middleware::Next,
    response::{AppendHeaders, IntoResponse, Response},
};
use std::sync::{Arc, Mutex};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::{
    error::{AppError, AppResult},
    security::{self, client_ip, ct_eq, random_token, sha256_hex},
    state::{AppState, Ceremony},
};

const SESSION_DAYS: i64 = 30;

/// Extractor: the request comes from the signed-in owner (valid session cookie).
/// For state-changing methods it also requires the `X-EOF` header, which a cross-site page
/// cannot set without a CORS preflight we never grant — that is our CSRF defence.
pub struct Owner {
    pub token_hash: String,
}

impl FromRequestParts<AppState> for Owner {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        if !matches!(parts.method, Method::GET | Method::HEAD) && !parts.headers.contains_key("x-eof") {
            return Err(AppError::Forbidden("missing X-EOF header".into()));
        }
        let token = session_cookie(&parts.headers, state.cfg.cookie_name()).ok_or(AppError::Unauthorized)?;
        let token_hash = sha256_hex(&token);
        let row: Option<(chrono::DateTime<Utc>,)> = sqlx::query_as(
            "SELECT last_seen_at FROM sessions WHERE token_hash = $1 AND expires_at > now()",
        )
        .bind(&token_hash)
        .fetch_optional(&state.db)
        .await?;
        let (last_seen,) = row.ok_or(AppError::Unauthorized)?;
        if Utc::now() - last_seen > Duration::minutes(10) {
            // Sliding expiry, written at most every 10 minutes.
            sqlx::query("UPDATE sessions SET last_seen_at = now(), expires_at = now() + make_interval(days => $2) WHERE token_hash = $1")
                .bind(&token_hash)
                .bind(SESSION_DAYS as i32)
                .execute(&state.db)
                .await?;
            // Ask `renew_cookie` to push the browser's Max-Age forward too, or the cookie
            // would still vanish 30 days after sign-in.
            if let Some(slot) = parts.extensions.get::<CookieRenewal>() {
                *slot.0.lock().unwrap() = Some(token);
            }
        }
        Ok(Owner { token_hash })
    }
}

/// Slot the `Owner` extractor fills when it slides a session's expiry.
#[derive(Clone, Default)]
struct CookieRenewal(Arc<Mutex<Option<String>>>);

/// Middleware: when a request slid its session forward, re-issue the cookie with a fresh
/// Max-Age so the browser keeps it as long as the server does.
pub async fn renew_cookie(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let slot = CookieRenewal::default();
    req.extensions_mut().insert(slot.clone());
    let mut res = next.run(req).await;
    let token = slot.0.lock().unwrap().take();
    // A handler that set its own cookie (e.g. logout clearing it) wins.
    if let Some(token) = token
        && !res.headers().contains_key(header::SET_COOKIE)
        && let Ok(v) = HeaderValue::from_str(&set_cookie(&state, &token, SESSION_DAYS * 86400))
    {
        res.headers_mut().append(header::SET_COOKIE, v);
    }
    res
}

pub fn session_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

pub async fn is_signed_in(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = session_cookie(headers, state.cfg.cookie_name()) else { return false };
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions WHERE token_hash = $1 AND expires_at > now()")
        .bind(sha256_hex(&token))
        .fetch_one(&state.db)
        .await
        .map(|n| n > 0)
        .unwrap_or(false)
}

fn set_cookie(state: &AppState, value: &str, max_age: i64) -> String {
    let secure = if state.cfg.secure() { "; Secure" } else { "" };
    // Lax (not Strict) so that arriving from an OAuth redirect (claude.ai → /oauth/authorize)
    // still carries your session; mutating requests are protected by the X-EOF header instead.
    format!("{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}", state.cfg.cookie_name())
}

async fn create_session(state: &AppState, headers: &HeaderMap) -> AppResult<String> {
    let token = random_token(32);
    let label = device_label(headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or(""));
    sqlx::query(
        "INSERT INTO sessions (token_hash, label, expires_at) VALUES ($1, $2, now() + make_interval(days => $3))",
    )
    .bind(sha256_hex(&token))
    .bind(label)
    .bind(SESSION_DAYS as i32)
    .execute(&state.db)
    .await?;
    Ok(set_cookie(state, &token, SESSION_DAYS * 86400))
}

pub fn device_label(ua: &str) -> String {
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Chrome/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let os = if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("Mac OS X") {
        "Mac"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "unknown device"
    };
    format!("{browser} on {os}")
}

// ── Setup ──────────────────────────────────────────────────────────────────

async fn passkey_count(state: &AppState) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM passkeys").fetch_one(&state.db).await?)
}

/// On boot with no passkeys enrolled, print a one-time setup code to the log.
pub async fn announce_setup(state: &AppState) -> anyhow::Result<()> {
    if passkey_count(state).await.map_err(|e| anyhow::anyhow!(e.to_string()))? > 0 {
        return Ok(());
    }
    let code = state.cfg.setup_code.clone().unwrap_or_else(readable_code);
    tracing::warn!("");
    tracing::warn!("  ┌─────────────────────────────────────────────┐");
    tracing::warn!("  │  EOF first-run setup code:  {code:<16}│");
    tracing::warn!("  │  Open {:<38}│", state.cfg.origin());
    tracing::warn!("  └─────────────────────────────────────────────┘");
    *state.setup_code.lock().unwrap() = Some(code);
    Ok(())
}

fn readable_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut raw = [0u8; 12];
    rand::fill(&mut raw[..]);
    let chars: String = raw.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
    format!("{}-{}-{}", &chars[0..4], &chars[4..8], &chars[8..12])
}

async fn owner_handle(state: &AppState) -> AppResult<Uuid> {
    let existing: Option<Value> = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'owner_handle'")
        .fetch_optional(&state.db)
        .await?;
    if let Some(id) = existing.and_then(|v| v.as_str().and_then(|s| s.parse().ok())) {
        return Ok(id);
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO settings (key, value) VALUES ('owner_handle', $1) ON CONFLICT (key) DO NOTHING")
        .bind(json!(id.to_string()))
        .execute(&state.db)
        .await?;
    Ok(id)
}

async fn load_passkeys(state: &AppState) -> AppResult<Vec<(Uuid, Passkey)>> {
    let rows: Vec<(Uuid, Value)> = sqlx::query_as("SELECT id, passkey FROM passkeys").fetch_all(&state.db).await?;
    rows.into_iter()
        .map(|(id, v)| {
            serde_json::from_value::<Passkey>(v)
                .map(|p| (id, p))
                .map_err(|e| AppError::Other(anyhow::anyhow!("corrupt passkey: {e}")))
        })
        .collect()
}

// ── HTTP handlers ──────────────────────────────────────────────────────────

pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let needs_setup = passkey_count(&state).await? == 0;
    let signed_in = !needs_setup && is_signed_in(&state, &headers).await;
    Ok(Json(json!({ "signed_in": signed_in, "needs_setup": needs_setup })))
}

#[derive(Deserialize)]
pub struct RegisterStart {
    name: Option<String>,
    setup_code: Option<String>,
}

pub async fn register_start(State(state): State<AppState>, req: Request) -> AppResult<Json<Value>> {
    let peer = security::peer(&req);
    let headers = req.headers().clone();
    let ip = client_ip(&headers, peer);
    security::limit(&state, "register", &ip, 10, 60)?;
    let body: RegisterStart = parse_json(req).await?;

    let via_setup = passkey_count(&state).await? == 0;
    if via_setup {
        // Compare ignoring case, spaces and dashes so "abcd-efgh" == "ABCDEFGH".
        let norm = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_uppercase();
        if state.setup_code.lock().unwrap().is_none() {
            // No code yet (e.g. passkeys were cleared while running): issue one now, in the log.
            announce_setup(&state).await.map_err(AppError::Other)?;
        }
        let expected = state.setup_code.lock().unwrap().clone();
        let given = norm(&body.setup_code.unwrap_or_default());
        match expected {
            Some(code) if !given.is_empty() && ct_eq(&norm(&code), &given) => {}
            _ => {
                // Per-IP and global caps: X-Forwarded-For can be spoofed, the global one can't.
                security::limit(&state, "setup-fail", &ip, 5, 600)?;
                security::limit(&state, "setup-fail", "all", 30, 600)?;
                return Err(AppError::Forbidden("That setup code isn't right. It's printed in the server log.".into()));
            }
        }
    } else if !is_signed_in(&state, &headers).await {
        return Err(AppError::Unauthorized);
    }

    let existing = load_passkeys(&state).await?;
    let exclude: Vec<CredentialID> = existing.iter().map(|(_, p)| p.cred_id().clone()).collect();
    let handle = owner_handle(&state).await?;
    let (options, reg) = state
        .webauthn
        .start_passkey_registration(handle, "owner", "EOF", Some(exclude))
        .map_err(|e| AppError::bad(format!("could not start registration: {e}")))?;
    let name = body
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| device_label(headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("")));
    let ceremony = state.put_ceremony(Ceremony::Register { state: reg, name, via_setup });
    Ok(Json(json!({ "ceremony": ceremony, "options": options })))
}

#[derive(Deserialize)]
pub struct Finish<T> {
    ceremony: String,
    credential: T,
}

pub async fn register_finish(State(state): State<AppState>, req: Request) -> AppResult<impl IntoResponse> {
    let headers = req.headers().clone();
    let body: Finish<RegisterPublicKeyCredential> = parse_json(req).await?;
    let Some(Ceremony::Register { state: reg, name, via_setup }) = state.take_ceremony(&body.ceremony) else {
        return Err(AppError::bad("This request expired. Try again."));
    };
    if !via_setup && !is_signed_in(&state, &headers).await {
        return Err(AppError::Unauthorized);
    }
    let passkey = state
        .webauthn
        .finish_passkey_registration(&body.credential, &reg)
        .map_err(|e| AppError::bad(format!("Passkey wasn't accepted: {e}")))?;
    sqlx::query("INSERT INTO passkeys (id, name, passkey) VALUES ($1, $2, $3)")
        .bind(Uuid::new_v4())
        .bind(&name)
        .bind(serde_json::to_value(&passkey).map_err(anyhow::Error::from)?)
        .execute(&state.db)
        .await?;
    let mut cookies = Vec::new();
    if via_setup {
        *state.setup_code.lock().unwrap() = None;
        tracing::info!("first passkey enrolled; setup code retired");
        cookies.push((header::SET_COOKIE, create_session(&state, &headers).await?));
    }
    Ok((AppendHeaders(cookies), Json(json!({ "ok": true, "name": name }))))
}

pub async fn login_start(State(state): State<AppState>, req: Request) -> AppResult<Json<Value>> {
    let ip = client_ip(req.headers(), security::peer(&req));
    security::limit(&state, "login", &ip, 20, 60)?;
    let keys: Vec<Passkey> = load_passkeys(&state).await?.into_iter().map(|(_, p)| p).collect();
    if keys.is_empty() {
        return Err(AppError::bad("No passkeys yet — finish setup first."));
    }
    let (options, auth) = state
        .webauthn
        .start_passkey_authentication(&keys)
        .map_err(|e| AppError::bad(format!("could not start sign-in: {e}")))?;
    let ceremony = state.put_ceremony(Ceremony::Login { state: auth });
    Ok(Json(json!({ "ceremony": ceremony, "options": options })))
}

pub async fn login_finish(State(state): State<AppState>, req: Request) -> AppResult<impl IntoResponse> {
    let headers = req.headers().clone();
    let ip = client_ip(&headers, security::peer(&req));
    security::limit(&state, "login", &ip, 20, 60)?;
    let body: Finish<PublicKeyCredential> = parse_json(req).await?;
    let Some(Ceremony::Login { state: auth }) = state.take_ceremony(&body.ceremony) else {
        return Err(AppError::bad("This request expired. Try again."));
    };
    let result = state
        .webauthn
        .finish_passkey_authentication(&body.credential, &auth)
        .map_err(|_| AppError::Forbidden("That passkey didn't check out.".into()))?;

    // Persist the updated signature counter / backup state.
    for (id, mut pk) in load_passkeys(&state).await? {
        if pk.cred_id() == result.cred_id() {
            pk.update_credential(&result);
            sqlx::query("UPDATE passkeys SET passkey = $2, last_used_at = now() WHERE id = $1")
                .bind(id)
                .bind(serde_json::to_value(&pk).map_err(anyhow::Error::from)?)
                .execute(&state.db)
                .await?;
        }
    }
    let cookie = create_session(&state, &headers).await?;
    Ok((AppendHeaders([(header::SET_COOKIE, cookie)]), Json(json!({ "ok": true }))))
}

pub async fn logout(State(state): State<AppState>, owner: Owner) -> AppResult<impl IntoResponse> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(&owner.token_hash).execute(&state.db).await?;
    Ok((AppendHeaders([(header::SET_COOKIE, set_cookie(&state, "", 0))]), Json(json!({ "ok": true }))))
}

// ── Device management (signed in) ──────────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct PasskeyRow {
    id: Uuid,
    name: String,
    created_at: chrono::DateTime<Utc>,
    last_used_at: Option<chrono::DateTime<Utc>>,
}

pub async fn list_passkeys(State(state): State<AppState>, _o: Owner) -> AppResult<Json<Vec<PasskeyRow>>> {
    Ok(Json(
        sqlx::query_as("SELECT id, name, created_at, last_used_at FROM passkeys ORDER BY created_at")
            .fetch_all(&state.db)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct Rename {
    name: String,
}

pub async fn rename_passkey(
    State(state): State<AppState>,
    _o: Owner,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(body): Json<Rename>,
) -> AppResult<Json<Value>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::bad("name can't be empty"));
    }
    sqlx::query("UPDATE passkeys SET name = $2 WHERE id = $1").bind(id).bind(name).execute(&state.db).await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_passkey(
    State(state): State<AppState>,
    _o: Owner,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> AppResult<Json<Value>> {
    if passkey_count(&state).await? <= 1 {
        return Err(AppError::bad("That's your only passkey — add another before removing it."));
    }
    sqlx::query("DELETE FROM passkeys WHERE id = $1").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct SessionRow {
    id: String,
    label: String,
    created_at: chrono::DateTime<Utc>,
    last_seen_at: chrono::DateTime<Utc>,
    #[sqlx(default)]
    current: bool,
}

pub async fn list_sessions(State(state): State<AppState>, owner: Owner) -> AppResult<Json<Vec<SessionRow>>> {
    let mut rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT left(token_hash, 16) AS id, label, created_at, last_seen_at FROM sessions WHERE expires_at > now() ORDER BY last_seen_at DESC",
    )
    .fetch_all(&state.db)
    .await?;
    for r in &mut rows {
        r.current = owner.token_hash.starts_with(&r.id);
    }
    Ok(Json(rows))
}

pub async fn delete_session(
    State(state): State<AppState>,
    _o: Owner,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> AppResult<Json<Value>> {
    if id.len() != 16 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::bad("bad session id"));
    }
    sqlx::query("DELETE FROM sessions WHERE left(token_hash, 16) = $1").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn parse_json<T: serde::de::DeserializeOwned>(req: Request) -> AppResult<T> {
    if !req.headers().contains_key("x-eof") {
        return Err(AppError::Forbidden("missing X-EOF header".into()));
    }
    let bytes = axum::body::to_bytes(req.into_body(), 64 * 1024)
        .await
        .map_err(|_| AppError::bad("body too large"))?;
    serde_json::from_slice(&bytes).map_err(|e| AppError::bad(format!("invalid JSON: {e}")))
}

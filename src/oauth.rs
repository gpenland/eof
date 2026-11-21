//! OAuth 2.1 authorization server for the MCP endpoint.
//!
//! MCP hosts (Claude.ai connectors, Claude Code, ...) discover us via the well-known metadata,
//! register themselves (RFC 7591), send you to /oauth/authorize, where — after a passkey
//! sign-in if needed — you approve the connection on a page that looks like the rest of EOF.
//! PKCE (S256) is mandatory; clients are public; tokens are opaque and stored hashed.
//!
//! For scripts and scheduled jobs, Settings can also mint a long-lived personal token.

use axum::{
    Form, Json,
    extract::{FromRequestParts, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Redirect, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

use crate::{
    auth::{self, Owner},
    error::{AppError, AppResult},
    events::Actor,
    security::{self, client_ip, random_token, sha256_b64url, sha256_hex},
    state::AppState,
};

const ACCESS_TTL_SECS: i64 = 60 * 60 * 24; // 1 day
const REFRESH_TTL_DAYS: i32 = 90;

// ── Discovery ──────────────────────────────────────────────────────────────

pub async fn protected_resource(State(st): State<AppState>) -> Json<Value> {
    let o = st.cfg.origin();
    Json(json!({
        "resource": format!("{o}/mcp"),
        "authorization_servers": [o],
        "bearer_methods_supported": ["header"],
        "scopes_supported": ["eof"],
        "resource_name": "EOF",
    }))
}

pub async fn authorization_server(State(st): State<AppState>) -> Json<Value> {
    let o = st.cfg.origin();
    Json(json!({
        "issuer": o,
        "authorization_endpoint": format!("{o}/oauth/authorize"),
        "token_endpoint": format!("{o}/oauth/token"),
        "registration_endpoint": format!("{o}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["eof"],
    }))
}

// ── Dynamic client registration ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterReq {
    client_name: Option<String>,
    redirect_uris: Vec<String>,
}

fn valid_redirect(u: &str) -> bool {
    match Url::parse(u) {
        Ok(url) => match url.scheme() {
            "https" => true,
            "http" => matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
            // Native apps may use private-use schemes (e.g. cursor://, vscode://).
            s => s.contains('.') || matches!(s, "cursor" | "vscode" | "claude"),
        },
        Err(_) => false,
    }
}

pub async fn register(State(st): State<AppState>, headers: HeaderMap, Json(req): Json<RegisterReq>) -> AppResult<Response> {
    // Per-IP and global caps: X-Forwarded-For can be spoofed, the global one can't.
    security::limit(&st, "oauth-register", &client_ip(&headers, None), 10, 3600)?;
    security::limit(&st, "oauth-register", "all", 100, 3600)?;
    if req.redirect_uris.is_empty() || req.redirect_uris.len() > 10 || !req.redirect_uris.iter().all(|u| valid_redirect(u)) {
        return Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri", "redirect_uris must be https or loopback"));
    }
    let name: String = req.client_name.unwrap_or_else(|| "AI assistant".into()).chars().take(80).collect();
    let client_id = format!("eofc_{}", random_token(16));
    sqlx::query("INSERT INTO oauth_clients (client_id, name, redirect_uris) VALUES ($1, $2, $3)")
        .bind(&client_id)
        .bind(&name)
        .bind(&req.redirect_uris)
        .execute(&st.db)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "client_id": client_id,
            "client_name": name,
            "redirect_uris": req.redirect_uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        })),
    )
        .into_response())
}

fn oauth_error(status: StatusCode, code: &str, desc: &str) -> Response {
    (status, Json(json!({ "error": code, "error_description": desc }))).into_response()
}

// ── Authorization ──────────────────────────────────────────────────────────

#[derive(Deserialize, Serialize, Clone)]
pub struct AuthorizeParams {
    response_type: Option<String>,
    client_id: String,
    redirect_uri: String,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    state: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}

async fn client(st: &AppState, id: &str) -> AppResult<Option<(String, Vec<String>)>> {
    Ok(sqlx::query_as("SELECT name, redirect_uris FROM oauth_clients WHERE client_id = $1")
        .bind(id)
        .fetch_optional(&st.db)
        .await?)
}

/// GET /oauth/authorize — validate, then hand off to the in-app consent screen.
pub async fn authorize(State(st): State<AppState>, headers: HeaderMap, Query(p): Query<AuthorizeParams>) -> AppResult<Response> {
    let Some((_, uris)) = client(&st, &p.client_id).await? else {
        return Ok((StatusCode::BAD_REQUEST, "Unknown client. Remove and re-add the connector.").into_response());
    };
    if !uris.contains(&p.redirect_uri) {
        return Ok((StatusCode::BAD_REQUEST, "redirect_uri doesn't match the registered client.").into_response());
    }
    if p.response_type.as_deref() != Some("code")
        || p.code_challenge.as_deref().unwrap_or("").len() < 43
        || p.code_challenge_method.as_deref() != Some("S256")
    {
        let mut u = Url::parse(&p.redirect_uri).map_err(|_| AppError::bad("bad redirect"))?;
        u.query_pairs_mut().append_pair("error", "invalid_request").append_pair("error_description", "PKCE S256 and response_type=code are required");
        if let Some(s) = &p.state {
            u.query_pairs_mut().append_pair("state", s);
        }
        return Ok(Redirect::to(u.as_str()).into_response());
    }
    let qs = serde_urlencoded(&p);
    let connect = format!("/connect?{qs}");
    if auth::is_signed_in(&st, &headers).await {
        Ok(Redirect::to(&connect).into_response())
    } else {
        let next: String = url::form_urlencoded::byte_serialize(connect.as_bytes()).collect();
        Ok(Redirect::to(&format!("/login?next={next}")).into_response())
    }
}

fn serde_urlencoded(p: &AuthorizeParams) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    let v = serde_json::to_value(p).unwrap_or_default();
    if let Some(obj) = v.as_object() {
        for (k, v) in obj {
            if let Some(v) = v.as_str() {
                s.append_pair(k, v);
            }
        }
    }
    s.finish()
}

pub async fn client_info(State(st): State<AppState>, _o: Owner, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let (name, uris) = client(&st, &id).await?.ok_or(AppError::NotFound("client"))?;
    let hosts: Vec<String> = uris.iter().filter_map(|u| Url::parse(u).ok()?.host_str().map(String::from)).collect();
    Ok(Json(json!({ "name": name, "redirect_hosts": hosts })))
}

#[derive(Deserialize)]
pub struct Approve {
    #[serde(flatten)]
    params: AuthorizeParams,
    allow: bool,
}

/// POST /api/oauth/approve — the signed-in owner pressed Allow (or Deny) on the consent screen.
pub async fn approve(State(st): State<AppState>, _o: Owner, Json(b): Json<Approve>) -> AppResult<Json<Value>> {
    let p = b.params;
    let (_, uris) = client(&st, &p.client_id).await?.ok_or(AppError::NotFound("client"))?;
    if !uris.contains(&p.redirect_uri) {
        return Err(AppError::bad("redirect_uri mismatch"));
    }
    let mut u = Url::parse(&p.redirect_uri).map_err(|_| AppError::bad("bad redirect"))?;
    if b.allow {
        let challenge = p.code_challenge.clone().filter(|c| c.len() >= 43).ok_or(AppError::bad("PKCE required"))?;
        let code = random_token(32);
        sqlx::query(
            "INSERT INTO oauth_codes (code_hash, client_id, redirect_uri, code_challenge, expires_at)
             VALUES ($1, $2, $3, $4, now() + interval '2 minutes')",
        )
        .bind(sha256_hex(&code))
        .bind(&p.client_id)
        .bind(&p.redirect_uri)
        .bind(challenge)
        .execute(&st.db)
        .await?;
        u.query_pairs_mut().append_pair("code", &code);
    } else {
        u.query_pairs_mut().append_pair("error", "access_denied");
    }
    if let Some(s) = &p.state {
        u.query_pairs_mut().append_pair("state", s);
    }
    u.query_pairs_mut().append_pair("iss", &st.cfg.origin());
    Ok(Json(json!({ "redirect": u.as_str() })))
}

// ── Token endpoint ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TokenReq {
    grant_type: String,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
}

async fn issue_pair(st: &AppState, grant_id: Uuid) -> AppResult<Value> {
    let access = format!("eof_at_{}", random_token(32));
    let refresh = format!("eof_rt_{}", random_token(32));
    let mut tx = st.db.begin().await?;
    sqlx::query("INSERT INTO tokens (token_hash, grant_id, kind, expires_at) VALUES ($1, $2, 'access', now() + make_interval(secs => $3))")
        .bind(sha256_hex(&access))
        .bind(grant_id)
        .bind(ACCESS_TTL_SECS as f64)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO tokens (token_hash, grant_id, kind, expires_at) VALUES ($1, $2, 'refresh', now() + make_interval(days => $3))")
        .bind(sha256_hex(&refresh))
        .bind(grant_id)
        .bind(REFRESH_TTL_DAYS)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": ACCESS_TTL_SECS,
        "refresh_token": refresh,
        "scope": "eof",
    }))
}

pub async fn token(State(st): State<AppState>, headers: HeaderMap, Form(req): Form<TokenReq>) -> AppResult<Response> {
    security::limit(&st, "oauth-token", &client_ip(&headers, None), 60, 60)?;
    let bad = |d: &str| Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", d));
    match req.grant_type.as_str() {
        "authorization_code" => {
            let (Some(code), Some(verifier), Some(client_id)) = (req.code, req.code_verifier, req.client_id) else {
                return Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "code, code_verifier and client_id are required"));
            };
            // Single use: delete-returning.
            let row: Option<(String, String, String)> = sqlx::query_as(
                "DELETE FROM oauth_codes WHERE code_hash = $1 AND expires_at > now() RETURNING client_id, redirect_uri, code_challenge",
            )
            .bind(sha256_hex(&code))
            .fetch_optional(&st.db)
            .await?;
            let Some((cid, ruri, challenge)) = row else { return bad("code is invalid or expired") };
            if cid != client_id || req.redirect_uri.as_deref().is_some_and(|r| r != ruri) {
                return bad("client or redirect mismatch");
            }
            if !security::ct_eq(&sha256_b64url(&verifier), &challenge) {
                return bad("PKCE verification failed");
            }
            let (name, _) = client(&st, &cid).await?.ok_or(AppError::NotFound("client"))?;
            let grant_id = Uuid::new_v4();
            sqlx::query("INSERT INTO grants (id, name, client_id) VALUES ($1, $2, $3)")
                .bind(grant_id)
                .bind(friendly_name(&name))
                .bind(&cid)
                .execute(&st.db)
                .await?;
            crate::events::emit(&st, "settings", &Actor(friendly_name(&name)), "connected to EOF");
            Ok(Json(issue_pair(&st, grant_id).await?).into_response())
        }
        "refresh_token" => {
            let Some(rt) = req.refresh_token else { return bad("refresh_token required") };
            // Rotate: the old refresh token (and that grant's access tokens) die now.
            let grant: Option<Uuid> = sqlx::query_scalar(
                "DELETE FROM tokens WHERE token_hash = $1 AND kind = 'refresh' AND expires_at > now() RETURNING grant_id",
            )
            .bind(sha256_hex(&rt))
            .fetch_optional(&st.db)
            .await?;
            let Some(grant_id) = grant else { return bad("refresh token is invalid or expired") };
            sqlx::query("DELETE FROM tokens WHERE grant_id = $1 AND kind = 'access'").bind(grant_id).execute(&st.db).await?;
            Ok(Json(issue_pair(&st, grant_id).await?).into_response())
        }
        _ => Ok(oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type", "use authorization_code or refresh_token")),
    }
}

/// "Claude Code (eof)" → "Claude Code"; keeps provenance labels short.
fn friendly_name(n: &str) -> String {
    let n = n.split(['(', '-', '|']).next().unwrap_or(n).trim();
    if n.is_empty() { "AI assistant".into() } else { n.chars().take(40).collect() }
}

// ── Bearer extractor for /mcp ──────────────────────────────────────────────

pub struct Machine {
    pub actor: Actor,
}

pub struct BearerRejection(String);

impl IntoResponse for BearerRejection {
    fn into_response(self) -> Response {
        let mut r = (StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid_token" }))).into_response();
        let v = format!(r#"Bearer resource_metadata="{}/.well-known/oauth-protected-resource""#, self.0);
        if let Ok(v) = HeaderValue::from_str(&v) {
            r.headers_mut().insert(header::WWW_AUTHENTICATE, v);
        }
        r
    }
}

impl FromRequestParts<AppState> for Machine {
    type Rejection = BearerRejection;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, BearerRejection> {
        let reject = || BearerRejection(st.cfg.origin());
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
            .ok_or_else(reject)?
            .trim();
        let row: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT g.id, g.name FROM tokens t JOIN grants g ON g.id = t.grant_id
             WHERE t.token_hash = $1 AND t.kind = 'access' AND (t.expires_at IS NULL OR t.expires_at > now())",
        )
        .bind(sha256_hex(token))
        .fetch_optional(&st.db)
        .await
        .map_err(|_| reject())?;
        let (gid, name) = row.ok_or_else(reject)?;
        let _ = sqlx::query("UPDATE grants SET last_used_at = now() WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')")
            .bind(gid)
            .execute(&st.db)
            .await;
        Ok(Machine { actor: Actor(name) })
    }
}

// ── Connections management (Settings) ──────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct GrantRow {
    id: Uuid,
    name: String,
    kind: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

pub async fn list_grants(State(st): State<AppState>, _o: Owner) -> AppResult<Json<Vec<GrantRow>>> {
    Ok(Json(
        sqlx::query_as(
            "SELECT id, name, CASE WHEN client_id IS NULL THEN 'token' ELSE 'oauth' END AS kind, created_at, last_used_at
             FROM grants ORDER BY coalesce(last_used_at, created_at) DESC",
        )
        .fetch_all(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
pub struct NewToken {
    name: String,
}

pub async fn create_personal_token(State(st): State<AppState>, _o: Owner, Json(b): Json<NewToken>) -> AppResult<Json<Value>> {
    let name: String = b.name.trim().chars().take(40).collect();
    if name.is_empty() {
        return Err(AppError::bad("give the token a name, like “Morning news job”"));
    }
    let gid = Uuid::new_v4();
    let token = format!("eof_pt_{}", random_token(32));
    sqlx::query("INSERT INTO grants (id, name) VALUES ($1, $2)").bind(gid).bind(&name).execute(&st.db).await?;
    sqlx::query("INSERT INTO tokens (token_hash, grant_id, kind) VALUES ($1, $2, 'access')")
        .bind(sha256_hex(&token))
        .bind(gid)
        .execute(&st.db)
        .await?;
    // Shown exactly once.
    Ok(Json(json!({ "id": gid, "name": name, "token": token })))
}

pub async fn delete_grant(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    sqlx::query("DELETE FROM grants WHERE id = $1").bind(id).execute(&st.db).await?;
    Ok(Json(json!({ "ok": true })))
}

//! Web Push: notifications to the installed app (iPhone Home Screen, desktop PWA, any browser).
//!
//! Self-contained, no push SDK: VAPID (RFC 8292) signs a short JWT with a P-256 key that EOF
//! generates once and keeps in `settings`; payloads are encrypted for each browser with
//! `aes128gcm` (RFC 8291 / RFC 8188). Push services that answer 404/410 have forgotten the
//! subscription, so it is deleted.

use std::time::Duration;

use aes_gcm::{Aes128Gcm, KeyInit, aead::Aead};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use chrono::{DateTime, Timelike, Utc};
use hkdf::Hkdf;
use p256::{
    PublicKey, SecretKey,
    ecdsa::{Signature, SigningKey, signature::Signer},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;
use sqlx::PgPool;

use crate::{
    actions,
    domain::{self, countdowns},
    error::{AppError, AppResult},
    security::sha256_hex,
    state::AppState,
};

/// What a notification says and where tapping it goes.
#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub title: String,
    pub body: String,
    /// Path inside the app, e.g. `/news`.
    pub url: String,
    /// Same tag replaces an earlier notification instead of stacking.
    pub tag: String,
}

#[derive(Debug, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    pub keys: SubKeys,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SubKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Device {
    /// Short hash of the endpoint (the endpoint itself is a capability and never leaves the server).
    pub id: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub last_ok_at: Option<DateTime<Utc>>,
}

// ── Keys ────────────────────────────────────────────────────────────────────

/// The server's VAPID key, created on first use and stored in `settings`.
pub async fn vapid_key(db: &PgPool) -> AppResult<SecretKey> {
    let stored: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'vapid_private'").fetch_optional(db).await?;
    if let Some(k) = stored.as_ref().and_then(|v| v.as_str()).and_then(|s| B64.decode(s).ok()).and_then(|b| SecretKey::from_slice(&b).ok()) {
        return Ok(k);
    }
    let key = loop {
        let mut b = [0u8; 32];
        rand::fill(&mut b);
        if let Ok(k) = SecretKey::from_slice(&b) {
            break k;
        }
    };
    // Two first-time callers may race; whoever lands first wins and both read it back.
    sqlx::query("INSERT INTO settings (key, value) VALUES ('vapid_private', $1) ON CONFLICT (key) DO NOTHING")
        .bind(json!(B64.encode(key.to_bytes())))
        .execute(db)
        .await?;
    Box::pin(vapid_key(db)).await
}

/// The public half, base64url uncompressed point: the browser's `applicationServerKey`.
pub fn public_key_b64(key: &SecretKey) -> String {
    B64.encode(key.public_key().to_sec1_bytes())
}

// ── Subscriptions ───────────────────────────────────────────────────────────

pub async fn subscribe(db: &PgPool, s: Subscription) -> AppResult<Device> {
    let url = url::Url::parse(&s.endpoint).map_err(|_| AppError::bad("endpoint must be a URL"))?;
    if url.scheme() != "https" {
        return Err(AppError::bad("endpoint must be https"));
    }
    let p256dh = B64.decode(s.keys.p256dh.trim_end_matches('=')).map_err(|_| AppError::bad("bad p256dh key"))?;
    PublicKey::from_sec1_bytes(&p256dh).map_err(|_| AppError::bad("bad p256dh key"))?;
    let auth = B64.decode(s.keys.auth.trim_end_matches('=')).map_err(|_| AppError::bad("bad auth secret"))?;
    if auth.len() != 16 {
        return Err(AppError::bad("bad auth secret"));
    }
    let label: String = s.label.unwrap_or_else(|| "This device".into()).chars().take(60).collect();
    sqlx::query(
        "INSERT INTO push_subscriptions (endpoint, p256dh, auth, label) VALUES ($1, $2, $3, $4)
         ON CONFLICT (endpoint) DO UPDATE SET p256dh = $2, auth = $3, label = $4",
    )
    .bind(&s.endpoint)
    .bind(B64.encode(&p256dh))
    .bind(B64.encode(&auth))
    .bind(&label)
    .execute(db)
    .await?;
    Ok(Device { id: device_id(&s.endpoint), label, created_at: Utc::now(), last_ok_at: None })
}

fn device_id(endpoint: &str) -> String {
    sha256_hex(endpoint)[..16].to_string()
}

pub async fn devices(db: &PgPool) -> AppResult<Vec<Device>> {
    let rows: Vec<Device> =
        sqlx::query_as("SELECT endpoint AS id, label, created_at, last_ok_at FROM push_subscriptions ORDER BY created_at")
            .fetch_all(db)
            .await?;
    Ok(rows.into_iter().map(|d| Device { id: device_id(&d.id), ..d }).collect())
}

/// Remove by device id or by the raw endpoint (a browser unsubscribing itself).
pub async fn remove(db: &PgPool, id_or_endpoint: &str) -> AppResult<()> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT endpoint FROM push_subscriptions").fetch_all(db).await?;
    for e in rows.iter().filter(|e| *e == id_or_endpoint || device_id(e) == id_or_endpoint) {
        sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = $1").bind(e).execute(db).await?;
    }
    Ok(())
}

// ── Sending ─────────────────────────────────────────────────────────────────

/// Encrypt `payload` for one browser (RFC 8291, aes128gcm content coding).
fn encrypt(ua_public: &[u8], auth: &[u8], payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    let ua = PublicKey::from_sec1_bytes(ua_public)?;
    let mut b = [0u8; 32];
    let eph = loop {
        rand::fill(&mut b);
        if let Ok(k) = SecretKey::from_slice(&b) {
            break k;
        }
    };
    let as_public = eph.public_key().to_sec1_bytes();
    let shared = p256::ecdh::diffie_hellman(eph.to_nonzero_scalar(), ua.as_affine());

    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(&as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .map_err(|_| anyhow::anyhow!("hkdf"))?;

    let mut salt = [0u8; 16];
    rand::fill(&mut salt);
    let prk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek).map_err(|_| anyhow::anyhow!("hkdf"))?;
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce).map_err(|_| anyhow::anyhow!("hkdf"))?;

    let mut plain = payload.to_vec();
    plain.push(0x02); // last (and only) record, no padding
    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|_| anyhow::anyhow!("aes key"))?;
    let sealed = cipher.encrypt(&nonce.into(), plain.as_slice()).map_err(|_| anyhow::anyhow!("aes-gcm"))?;

    let mut body = Vec::with_capacity(86 + sealed.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&4096u32.to_be_bytes());
    body.push(as_public.len() as u8);
    body.extend_from_slice(&as_public);
    body.extend_from_slice(&sealed);
    Ok(body)
}

/// `Authorization: vapid t=…, k=…` for one push service origin.
fn vapid_header(key: &SecretKey, endpoint: &str, subject: &str) -> anyhow::Result<String> {
    let u = url::Url::parse(endpoint)?;
    let aud = u.origin().ascii_serialization();
    let exp = Utc::now().timestamp() + 12 * 3600;
    let header = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = B64.encode(json!({ "aud": aud, "exp": exp, "sub": subject }).to_string());
    let signing_input = format!("{header}.{claims}");
    let sig: Signature = SigningKey::from(key).sign(signing_input.as_bytes());
    Ok(format!("vapid t={signing_input}.{}, k={}", B64.encode(sig.to_bytes()), public_key_b64(key)))
}

/// Contact for push services. An https origin is accepted everywhere; localhost isn't public.
fn subject(st: &AppState) -> String {
    if st.cfg.secure() { st.cfg.origin() } else { "mailto:eof@example.com".into() }
}

/// Send to every subscribed device. Returns how many accepted it.
pub async fn send(st: &AppState, note: &Note) -> AppResult<usize> {
    let subs: Vec<(String, String, String)> =
        sqlx::query_as("SELECT endpoint, p256dh, auth FROM push_subscriptions").fetch_all(&st.db).await?;
    if subs.is_empty() {
        return Ok(0);
    }
    let key = vapid_key(&st.db).await?;
    let sub = subject(st);
    let payload = serde_json::to_vec(note).map_err(|e| AppError::Other(e.into()))?;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build().map_err(|e| AppError::Other(e.into()))?;
    let mut ok = 0;
    for (endpoint, p256dh, auth) in subs {
        match send_one(&client, &key, &sub, &endpoint, &p256dh, &auth, &payload).await {
            Outcome::Delivered => {
                ok += 1;
                let _ = sqlx::query("UPDATE push_subscriptions SET last_ok_at = now() WHERE endpoint = $1").bind(&endpoint).execute(&st.db).await;
            }
            Outcome::Gone => {
                tracing::info!("push subscription expired; removing it");
                let _ = sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = $1").bind(&endpoint).execute(&st.db).await;
            }
            Outcome::Failed => {} // logged in send_one; kept for the next try
        }
    }
    Ok(ok)
}

enum Outcome {
    Delivered,
    /// The push service forgot this subscription (404/410): delete it.
    Gone,
    /// Anything else (network, 4xx/5xx, bad stored keys): logged, subscription kept.
    Failed,
}

async fn send_one(
    client: &reqwest::Client,
    key: &SecretKey,
    subject: &str,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    payload: &[u8],
) -> Outcome {
    let attempt = async {
        let body = encrypt(&B64.decode(p256dh)?, &B64.decode(auth)?, payload)?;
        let res = client
            .post(endpoint)
            .header("authorization", vapid_header(key, endpoint, subject)?)
            .header("content-encoding", "aes128gcm")
            .header("content-type", "application/octet-stream")
            .header("ttl", "86400")
            .header("urgency", "normal")
            .body(body)
            .send()
            .await?;
        anyhow::Ok(res)
    };
    match attempt.await {
        Ok(r) if r.status().is_success() => Outcome::Delivered,
        Ok(r) if matches!(r.status().as_u16(), 404 | 410) => Outcome::Gone,
        Ok(r) => {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            tracing::warn!("push service answered {status}: {}", text.chars().take(200).collect::<String>());
            Outcome::Failed
        }
        Err(e) => {
            tracing::warn!("push failed: {e:#}");
            Outcome::Failed
        }
    }
}

/// Fire-and-forget: notification problems never fail the action that caused them.
pub fn spawn_send(st: &AppState, note: Note) {
    let st = st.clone();
    tokio::spawn(async move {
        if let Err(e) = send(&st, &note).await {
            tracing::warn!("push: {e}");
        }
    });
}

// ── Morning summary ─────────────────────────────────────────────────────────

/// Local hour (0–23) for the daily summary, or `None` when it's off. Default 8.
pub async fn digest_hour(db: &PgPool) -> Option<u32> {
    let v: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'push_digest_hour'").fetch_optional(db).await.ok().flatten();
    match v {
        None => Some(8),
        Some(serde_json::Value::Null) => None,
        Some(v) => v.as_u64().map(|h| h.min(23) as u32),
    }
}

pub async fn set_digest_hour(db: &PgPool, hour: Option<u32>) -> AppResult<()> {
    if hour.is_some_and(|h| h > 23) {
        return Err(AppError::bad("hour must be 0–23"));
    }
    sqlx::query("INSERT INTO settings (key, value) VALUES ('push_digest_hour', $1) ON CONFLICT (key) DO UPDATE SET value = $1")
        .bind(json!(hour))
        .execute(db)
        .await?;
    Ok(())
}

/// The morning summary, or `None` when there's nothing worth a buzz.
pub async fn digest(st: &AppState) -> AppResult<Option<Note>> {
    let o = actions::overview(st).await?;
    let n = |k: &str| o["tasks"][k].as_array().map_or(0, Vec::len);
    let (overdue, today) = (n("overdue"), n("today"));
    let mut parts = Vec::new();
    if today > 0 {
        parts.push(format!("{today} due today"));
    }
    if overdue > 0 {
        parts.push(format!("{overdue} overdue"));
    }
    let cds: Vec<countdowns::CountdownView> = {
        let tz = domain::timezone(&st.db).await;
        let t = domain::today(&st.db).await;
        countdowns::list(&st.db, false).await?.into_iter().map(|c| countdowns::view(c, tz, t)).collect()
    };
    for c in cds.iter().filter(|c| matches!(c.days_until, 0 | 1 | 7)).take(2) {
        parts.push(match c.days_until {
            0 => format!("{} is today", c.c.title),
            1 => format!("{} is tomorrow", c.c.title),
            d => format!("{} in {d} days", c.c.title),
        });
    }
    let first = o["tasks"]["overdue"].as_array().into_iter().chain(o["tasks"]["today"].as_array()).flatten().next();
    if parts.is_empty() {
        return Ok(None);
    }
    let edition = o["latest_issue"]["is_today"].as_bool() == Some(true);
    Ok(Some(Note {
        title: if edition { "Good morning — today’s edition is in".into() } else { "Good morning".into() },
        body: match first.and_then(|t| t["title"].as_str()) {
            Some(t) => format!("{} · first up: {t}", parts.join(" · ")),
            None => parts.join(" · "),
        },
        url: if edition { "/news".into() } else { "/tasks".into() },
        tag: "digest".into(),
    }))
}

/// Once a minute: at the chosen local hour, send the morning summary (once per day).
pub fn spawn_scheduler(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            if let Err(e) = maybe_digest(&st).await {
                tracing::warn!("morning summary: {e}");
            }
        }
    });
}

async fn maybe_digest(st: &AppState) -> AppResult<()> {
    let Some(hour) = digest_hour(&st.db).await else { return Ok(()) };
    let tz = domain::timezone(&st.db).await;
    let now = Utc::now().with_timezone(&tz);
    if now.hour() < hour {
        return Ok(());
    }
    let today = now.date_naive();
    // Claim the day first so restarts and slow sends can't double up.
    let claimed = sqlx::query(
        "INSERT INTO settings (key, value) VALUES ('push_digest_last', $1)
         ON CONFLICT (key) DO UPDATE SET value = $1 WHERE settings.value IS DISTINCT FROM $1",
    )
    .bind(json!(today))
    .execute(&st.db)
    .await?
    .rows_affected();
    if claimed == 0 {
        return Ok(());
    }
    let has_devices: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM push_subscriptions)").fetch_one(&st.db).await?;
    if has_devices && let Some(note) = digest(st).await? {
        send(st, &note).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::Aead;

    /// Decrypt our own output with the receiver's keys, the way a browser would.
    #[test]
    fn round_trips_rfc8291() {
        let ua = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let ua_pub = ua.public_key().to_sec1_bytes();
        let auth = [9u8; 16];
        let body = encrypt(&ua_pub, &auth, b"hello").unwrap();

        let (salt, rest) = body.split_at(16);
        assert_eq!(&rest[..4], &4096u32.to_be_bytes());
        let idlen = rest[4] as usize;
        let as_pub = &rest[5..5 + idlen];
        let sealed = &rest[5 + idlen..];

        let shared = p256::ecdh::diffie_hellman(ua.to_nonzero_scalar(), PublicKey::from_sec1_bytes(as_pub).unwrap().as_affine());
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(&ua_pub);
        info.extend_from_slice(as_pub);
        let mut ikm = [0u8; 32];
        Hkdf::<Sha256>::new(Some(&auth), shared.raw_secret_bytes()).expand(&info, &mut ikm).unwrap();
        let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
        let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
        prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek).unwrap();
        prk.expand(b"Content-Encoding: nonce\0", &mut nonce).unwrap();
        let plain = Aes128Gcm::new_from_slice(&cek).unwrap().decrypt(&nonce.into(), sealed).unwrap();
        assert_eq!(plain, b"hello\x02");
    }

    #[test]
    fn vapid_jwt_shape() {
        let k = SecretKey::from_slice(&[3u8; 32]).unwrap();
        let h = vapid_header(&k, "https://web.push.apple.com/abc", "https://eof.example").unwrap();
        assert!(h.starts_with("vapid t=eyJ"));
        assert!(h.contains(&format!(", k={}", public_key_b64(&k))));
        assert_eq!(B64.decode(public_key_b64(&k)).unwrap().len(), 65);
    }
}

//! Small security primitives: random tokens, hashing, rate limiting, response headers.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, header},
    middleware::Next,
    response::Response,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

use crate::state::AppState;

/// `bytes` of CSPRNG output, base64url encoded.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::fill(&mut buf[..]);
    URL_SAFE_NO_PAD.encode(&buf)
}

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

pub fn sha256_b64url(s: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(s.as_bytes()))
}

/// Constant-time string equality (for setup codes).
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Best-effort client IP. Sevalla (and most PaaS) put the real client first in X-Forwarded-For.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| peer.map(|p| p.ip().to_string()))
        .unwrap_or_else(|| "unknown".into())
}

/// Fixed-window limiter keyed by (bucket, ip).
#[derive(Default)]
pub struct RateLimiter {
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    /// Returns false if `key` exceeded `max` hits in the last `window`.
    pub fn check(&self, key: String, max: u32, window: Duration) -> bool {
        let mut map = self.windows.lock().unwrap();
        let entry = map.entry(key).or_insert((Instant::now(), 0));
        if entry.0.elapsed() > window {
            *entry = (Instant::now(), 0);
        }
        entry.1 += 1;
        entry.1 <= max
    }

    pub fn sweep(&self) {
        self.windows.lock().unwrap().retain(|_, (t, _)| t.elapsed() < Duration::from_secs(3600));
    }
}

pub fn limit(state: &AppState, bucket: &str, ip: &str, max: u32, secs: u64) -> Result<(), crate::error::AppError> {
    if state.limiter.check(format!("{bucket}:{ip}"), max, Duration::from_secs(secs)) {
        Ok(())
    } else {
        tracing::warn!("rate limited {bucket} for {ip}");
        Err(crate::error::AppError::RateLimited)
    }
}

pub fn peer(req: &Request) -> Option<SocketAddr> {
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0)
}

/// Strict security headers on every response.
pub async fn headers(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    let set = |h: &mut HeaderMap, k: &'static str, v: &'static str| {
        h.entry(k).or_insert(HeaderValue::from_static(v));
    };
    set(
        h,
        "content-security-policy",
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
         connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'",
    );
    set(h, "x-content-type-options", "nosniff");
    set(h, "referrer-policy", "same-origin");
    set(h, "cross-origin-opener-policy", "same-origin");
    set(h, "permissions-policy", "camera=(), microphone=(), geolocation=(), payment=()");
    if state.cfg.secure() {
        set(h, "strict-transport-security", "max-age=63072000; includeSubDomains");
    }
    if !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}

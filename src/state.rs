use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use sqlx::PgPool;
use tokio::sync::broadcast;
use webauthn_rs::prelude::*;

use crate::{config::Config, email::Mailer, events::Event, security::RateLimiter};

/// An in-flight WebAuthn ceremony. Kept server-side (never round-tripped through the client)
/// so a challenge can only be answered once.
pub enum Ceremony {
    Register { state: PasskeyRegistration, name: String, via_setup: bool },
    Login { state: PasskeyAuthentication },
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub db: PgPool,
    pub webauthn: Webauthn,
    pub events: broadcast::Sender<Event>,
    pub ceremonies: Mutex<HashMap<String, (Instant, Ceremony)>>,
    pub limiter: RateLimiter,
    /// First-run setup code; `None` once a passkey exists.
    pub setup_code: Mutex<Option<String>>,
    /// `None` when email isn't configured.
    pub mailer: Option<Mailer>,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub async fn new(cfg: Config, db: PgPool) -> anyhow::Result<Self> {
        let rp_id = cfg.public_url.host_str().expect("validated in config").to_string();
        let webauthn = WebauthnBuilder::new(&rp_id, &cfg.public_url)?
            .rp_name("EOF")
            .timeout(Duration::from_secs(120))
            .build()?;
        let (events, _) = broadcast::channel(64);
        let mailer = cfg.email.clone().map(Mailer::new).transpose()?;
        Ok(Self(Arc::new(Inner {
            cfg,
            db,
            webauthn,
            events,
            ceremonies: Mutex::new(HashMap::new()),
            limiter: RateLimiter::default(),
            setup_code: Mutex::new(None),
            mailer,
        })))
    }

    pub fn put_ceremony(&self, c: Ceremony) -> String {
        let id = crate::security::random_token(18);
        let mut map = self.ceremonies.lock().unwrap();
        map.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(300));
        map.insert(id.clone(), (Instant::now(), c));
        id
    }

    pub fn take_ceremony(&self, id: &str) -> Option<Ceremony> {
        let mut map = self.ceremonies.lock().unwrap();
        map.remove(id).filter(|(t, _)| t.elapsed() < Duration::from_secs(300)).map(|(_, c)| c)
    }
}

/// Periodically sweep expired sessions, codes and tokens.
pub fn spawn_janitor(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let db = &state.db;
            let _ = sqlx::query("DELETE FROM sessions WHERE expires_at < now()").execute(db).await;
            let _ = sqlx::query("DELETE FROM oauth_codes WHERE expires_at < now()").execute(db).await;
            let _ = sqlx::query("DELETE FROM tokens WHERE expires_at IS NOT NULL AND expires_at < now()").execute(db).await;
            // OAuth grants whose refresh tokens have all expired are dead connections.
            let _ = sqlx::query(
                "DELETE FROM grants g WHERE client_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM tokens t WHERE t.grant_id = g.id)",
            )
            .execute(db)
            .await;
            state.limiter.sweep();
        }
    });
}

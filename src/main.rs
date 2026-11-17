//! EOF — a small, private productivity server.
//!
//! One binary: Postgres-backed domain services (tasks, countdowns, news), a JSON API for the
//! web UI, an MCP endpoint so any AI can do everything the UI can, passkey auth for humans and
//! OAuth 2.1 for machines.

mod actions;
mod api;
mod auth;
mod config;
mod domain;
mod error;
mod events;
mod mcp;
mod oauth;
mod security;
mod state;
mod web;

use std::net::SocketAddr;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("eof=info,tower_http=warn")),
        )
        .init();

    let cfg = config::Config::from_env()?;

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&cfg.database_url)
        .await
        .context("connecting to Postgres (DATABASE_URL)")?;
    sqlx::migrate!("./migrations").run(&db).await.context("running migrations")?;
    domain::tasks::ensure_default_list(&db).await?;

    let state = state::AppState::new(cfg.clone(), db).await?;
    auth::announce_setup(&state).await?;
    state::spawn_janitor(state.clone());

    let app = web::router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!("EOF listening on {addr} — public URL {}", cfg.public_url);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

//! Router assembly and the embedded web UI.

use axum::{
    Router,
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rust_embed::RustEmbed;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::{actions, api, auth, capture, mcp, oauth, rest, security, state::AppState};

#[derive(RustEmbed)]
#[folder = "web/"]
struct Assets;

pub fn router(state: AppState) -> Router {
    // Machine-facing endpoints: bearer/no credentials, so open CORS is safe and lets
    // browser-based MCP clients connect.
    let machine = Router::new()
        .route("/mcp", post(mcp::handle).get(mcp::get_not_allowed))
        .route("/.well-known/oauth-protected-resource", get(oauth::protected_resource))
        .route("/.well-known/oauth-protected-resource/mcp", get(oauth::protected_resource))
        .route("/.well-known/oauth-authorization-server", get(oauth::authorization_server))
        .route("/.well-known/openid-configuration", get(oauth::authorization_server))
        .route("/oauth/register", post(oauth::register))
        .route("/oauth/token", post(oauth::token))
        .route("/capture", post(capture::handle))
        .route("/v1", get(rest::index))
        .route("/v1/{name}", post(rest::handle))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers(Any)
                .expose_headers([header::WWW_AUTHENTICATE, "mcp-session-id".parse().unwrap()]),
        );

    let auth_routes = Router::new()
        .route("/status", get(auth::status))
        .route("/register/start", post(auth::register_start))
        .route("/register/finish", post(auth::register_finish))
        .route("/login/start", post(auth::login_start))
        .route("/login/finish", post(auth::login_finish));

    Router::new()
        .merge(machine)
        .route("/oauth/authorize", get(oauth::authorize))
        .nest("/auth", auth_routes)
        .nest("/api", api::router())
        .route("/healthz", get(|| async { "ok" }))
        .route("/mail/read/{token}", get(mail_read))
        .fallback(static_or_app)
        .layer(middleware::from_fn_with_state(state.clone(), auth::renew_cookie))
        .layer(middleware::from_fn_with_state(state.clone(), security::headers))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The 1×1 image in an emailed edition. Loading it marks that edition read; it always answers
/// with the image (never an error) so a mail client has nothing to show or retry.
async fn mail_read(State(st): State<AppState>, Path(token): Path<String>) -> Response {
    if token.len() <= 64
        && let Err(e) = actions::read_issue_by_email(&st, &token).await
    {
        tracing::warn!("marking an emailed edition read: {e}");
    }
    // A transparent 1×1 GIF.
    const GIF: &[u8] = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\0\0\0!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;";
    ([(header::CONTENT_TYPE, "image/gif"), (header::CACHE_CONTROL, "no-store, private")], GIF).into_response()
}

const APP_ROUTES: [&str; 6] = ["/", "/tasks", "/countdown", "/news", "/settings", "/login"];

async fn static_or_app(req: Request) -> Response {
    let path = req.uri().path();
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }
    let is_app = APP_ROUTES.contains(&path) || path.starts_with("/news/") || path == "/connect";
    let file = if is_app { "index.html" } else { path.trim_start_matches('/') };
    let Some(asset) = Assets::get(file) else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    let etag = format!("\"{}\"", hex::encode(&asset.metadata.sha256_hash()[..12]));
    if req.headers().get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    let mime = mime_guess::from_path(file).first_or_octet_stream();
    let mut res = Response::new(Body::from(asset.data.into_owned()));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap());
    h.insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    res
}

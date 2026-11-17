use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("not signed in")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("too many attempts — wait a minute and try again")]
    RateLimited,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::Db(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
            Self::Db(_) | Self::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Message safe to show to a client; internal errors are logged, not leaked.
    pub fn public_message(&self) -> String {
        match self {
            Self::Db(sqlx::Error::RowNotFound) => "not found".into(),
            Self::Db(e) => {
                tracing::error!("database error: {e}");
                "internal error".into()
            }
            Self::Other(e) => {
                tracing::error!("internal error: {e:#}");
                "internal error".into()
            }
            other => other.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        (status, Json(json!({ "error": self.public_message() }))).into_response()
    }
}

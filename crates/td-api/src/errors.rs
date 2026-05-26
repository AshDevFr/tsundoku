//! Error type for handler responses.
//!
//! `anyhow::Error` is the default failure mode internally; converting it
//! into a JSON envelope at the handler boundary keeps individual handlers
//! single-purpose.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("service misconfigured: {0}")]
    Misconfigured(String),

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    pub fn status(&self) -> StatusCode {
        match self {
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden => StatusCode::FORBIDDEN,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::Misconfigured(_) => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorBody {
    pub error: String,
    pub message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let (code, message) = match &self {
            ApiError::NotFound(msg) => ("not_found", msg.clone()),
            ApiError::BadRequest(msg) => ("bad_request", msg.clone()),
            ApiError::Unauthorized => ("unauthorized", "missing or invalid credentials".into()),
            ApiError::Forbidden => ("forbidden", "credentials lack the required scope".into()),
            ApiError::Conflict(msg) => ("conflict", msg.clone()),
            ApiError::Misconfigured(msg) => ("misconfigured", msg.clone()),
            ApiError::Internal(e) => {
                tracing::error!(error = ?e, "handler error");
                ("internal", "internal server error".into())
            }
        };
        (
            status,
            Json(ApiErrorBody {
                error: code.into(),
                message,
            }),
        )
            .into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

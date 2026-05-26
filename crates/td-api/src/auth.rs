//! Auth middleware.
//!
//! Two layers:
//!
//! - **API key** (`X-API-Key` header or `Authorization: Bearer …`). Gates
//!   the read endpoints when `auth.read_requires_auth = true`. When the
//!   config flag is false, the middleware short-circuits to "allow".
//! - **Admin bearer** (`Authorization: Bearer …`). Gates every write
//!   endpoint. If `auth.admin_token` is unset the middleware refuses every
//!   request with 503 (a fresh install must opt in by setting a token).
//!
//! The two checks live in separate middleware functions rather than one
//! big role-gate because the bearer-header consumer and the API-key
//! consumer overlap (the read flag also accepts a bearer) and a single
//! function would have to encode the read/write distinction in its
//! parameters anyway.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, Request, header};
use axum::middleware::Next;
use axum::response::Response;
use td_config::AuthConfig;

use crate::errors::ApiError;

const API_KEY_HEADER: &str = "x-api-key";

/// Layer for `/api/v1/*` reads. Allows through when
/// `auth.read_requires_auth = false`; otherwise demands either
/// `X-API-Key: <api_key>` or `Authorization: Bearer <api_key>`.
pub async fn require_read(
    State(auth): State<Arc<AuthConfig>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, ApiError> {
    if !auth.read_requires_auth {
        return Ok(next.run(request).await);
    }
    let expected = auth.api_key.as_deref().ok_or_else(|| {
        ApiError::Misconfigured(
            "auth.api_key is required when auth.read_requires_auth is true".into(),
        )
    })?;
    let supplied = extract_api_key(request.headers());
    if supplied.as_deref() == Some(expected) {
        Ok(next.run(request).await)
    } else {
        Err(ApiError::Unauthorized)
    }
}

/// Layer for `/api/v1/*` writes. Requires `Authorization: Bearer
/// <admin_token>`. If `auth.admin_token` is unset, every request is
/// refused with 503: the operator has not enabled writes yet.
pub async fn require_admin(
    State(auth): State<Arc<AuthConfig>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let expected = auth.admin_token.as_deref().ok_or_else(|| {
        ApiError::Misconfigured("auth.admin_token is unset; write endpoints disabled".into())
    })?;
    let supplied = extract_bearer(request.headers());
    if supplied.as_deref() == Some(expected) {
        Ok(next.run(request).await)
    } else {
        Err(ApiError::Unauthorized)
    }
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(|s| s.trim().to_string())
}

fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(API_KEY_HEADER)
        && let Ok(s) = v.to_str()
    {
        return Some(s.to_string());
    }
    extract_bearer(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn extracts_x_api_key_header() {
        let mut h = HeaderMap::new();
        h.insert(API_KEY_HEADER, HeaderValue::from_static("k1"));
        assert_eq!(extract_api_key(&h).as_deref(), Some("k1"));
    }

    #[test]
    fn extracts_bearer_from_authorization() {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc"),
        );
        assert_eq!(extract_bearer(&h).as_deref(), Some("abc"));
        assert_eq!(extract_api_key(&h).as_deref(), Some("abc"));
    }

    #[test]
    fn missing_headers_return_none() {
        let h = HeaderMap::new();
        assert!(extract_api_key(&h).is_none());
        assert!(extract_bearer(&h).is_none());
    }
}

//! HTTP API for tsundoku.
//!
//! Owns the axum router, auth middleware, request/response DTOs, and the
//! utoipa OpenAPI specification. The binary crate wires this together with
//! the discovery-source registry, metadata registry, and scheduler locks
//! built from config.
//!
//! Layering:
//! - [`router`] builds the top-level `axum::Router`. Reads mount under
//!   `/api/v1`; writes mount under the same prefix but layer an admin
//!   bearer check via [`auth::require_admin`].
//! - [`handlers`] are the per-resource modules. Each handler is a `utoipa`
//!   path so the OpenAPI spec stays in sync with the routes.
//! - [`embed`] serves the SPA from the binary when the `embed-frontend`
//!   feature is enabled. Without it, every non-API path returns a friendly
//!   404 explaining how to run the Vite dev server instead.

pub mod auth;
pub mod docs;
pub mod embed;
pub mod errors;
pub mod handlers;
pub mod router;
pub mod state;

pub use docs::ApiDoc;
pub use router::router;
pub use state::AppState;

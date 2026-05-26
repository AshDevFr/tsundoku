//! HTTP handlers. One module per resource.
//!
//! Each handler is a `utoipa::path` so the OpenAPI spec stays a derived
//! artifact: adding a route means adding a handler + registering it in
//! [`crate::docs::ApiDoc`].

pub mod health;
pub mod pagination;
pub mod providers;
pub mod releases;
pub mod series;
pub mod sources;
pub mod stats;

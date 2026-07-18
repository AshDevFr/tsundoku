//! HTTP handlers. One module per resource.
//!
//! Each handler is a `utoipa::path` so the OpenAPI spec stays a derived
//! artifact: adding a route means adding a handler + registering it in
//! [`crate::docs::ApiDoc`].

pub mod codex;
pub mod covers;
pub mod download;
pub mod events;
pub mod health;
pub mod info;
pub mod metrics;
pub mod pagination;
pub mod providers;
pub mod releases;
pub mod search;
pub mod series;
pub mod series_export;
pub mod sources;
pub mod stats;
pub mod tagging;

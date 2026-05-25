//! Storage layer for tsundoku.
//!
//! - [`conn`] opens the SQLite database and applies the journal-mode /
//!   foreign-key / busy-timeout pragmas the rest of the service expects.
//! - [`entities`] are the sea-orm models for the schema landed in `migration`.
//! - [`repos`] are thin typed query helpers consumed by the API and worker
//!   crates. They deliberately stay small; complex orchestration lives in the
//!   caller (resolution pipeline, scheduler) rather than here.

pub mod conn;
pub mod entities;
pub mod repos;

pub use conn::{connect, connect_in_memory, run_migrations};
pub use sea_orm::DatabaseConnection;

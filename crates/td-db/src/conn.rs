//! SQLite connection helper.
//!
//! SQLite pragmas like `foreign_keys` and `busy_timeout` are per-connection,
//! so applying them via a single `execute()` only sticks if every later query
//! uses that same connection. The pragmatic shape for a personal-scale service
//! (matching the PRD's "no concurrency story beyond serializing writes" line)
//! is a single-connection pool: pragmas are guaranteed to apply to every query
//! and SQLite's writer-serialization model is preserved. `journal_mode=WAL`
//! is a persistent file property and is set idempotently on connect.

use anyhow::Context;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement};
use td_config::AppConfig;

/// Default SQLite `busy_timeout` (ms) applied on connect.
pub const DEFAULT_BUSY_TIMEOUT_MS: u32 = 5_000;

pub async fn connect(cfg: &AppConfig) -> anyhow::Result<DatabaseConnection> {
    let paths = cfg.storage.paths();
    paths.ensure()?;
    let url = paths.database_url();

    let mut opts = ConnectOptions::new(&url);
    // SQLite serializes writers; multi-connection pools just multiply the
    // chance of `SQLITE_BUSY` without buying throughput at this scale.
    opts.max_connections(1).sqlx_logging(false);
    let db = Database::connect(opts)
        .await
        .with_context(|| format!("connecting to database {url}"))?;

    apply_sqlite_pragmas(&db).await?;
    Ok(db)
}

pub async fn run_migrations(db: &DatabaseConnection) -> anyhow::Result<()> {
    Migrator::up(db, None).await?;
    Ok(())
}

async fn apply_sqlite_pragmas(db: &DatabaseConnection) -> anyhow::Result<()> {
    let backend = db.get_database_backend();
    let pragmas = [
        "PRAGMA journal_mode=WAL;".to_string(),
        "PRAGMA foreign_keys=ON;".to_string(),
        format!("PRAGMA busy_timeout={DEFAULT_BUSY_TIMEOUT_MS};"),
    ];
    for p in &pragmas {
        db.execute(Statement::from_string(backend, p.clone()))
            .await
            .with_context(|| format!("applying {p}"))?;
    }
    Ok(())
}

/// Open an in-memory SQLite connection and run all migrations. For tests only.
pub async fn connect_in_memory() -> anyhow::Result<DatabaseConnection> {
    let db = Database::connect("sqlite::memory:")
        .await
        .context("opening in-memory sqlite")?;
    apply_sqlite_pragmas(&db).await?;
    run_migrations(&db).await?;
    Ok(db)
}

use anyhow::Context;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use td_config::AppConfig;

pub async fn connect(cfg: &AppConfig) -> anyhow::Result<DatabaseConnection> {
    let mut opts = ConnectOptions::new(&cfg.database.url);
    opts.max_connections(cfg.database.max_connections)
        .sqlx_logging(false);
    let db = Database::connect(opts)
        .await
        .with_context(|| format!("connecting to database {}", cfg.database.url))?;

    // SQLite: WAL is a persistent file property (set once), foreign-key
    // enforcement is per-connection. For a single-binary personal service the
    // pool is small enough that running these on connect is sufficient; revisit
    // with a per-connection hook if you scale the pool.
    if cfg.database.url.starts_with("sqlite") {
        use sea_orm::{ConnectionTrait, Statement};
        for pragma in ["PRAGMA journal_mode=WAL;", "PRAGMA foreign_keys=ON;"] {
            db.execute(Statement::from_string(db.get_database_backend(), pragma))
                .await?;
        }
    }
    Ok(db)
}

pub async fn run_migrations(db: &DatabaseConnection) -> anyhow::Result<()> {
    Migrator::up(db, None).await?;
    Ok(())
}

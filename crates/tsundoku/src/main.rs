mod commands;
mod http_limiter;
mod metadata;
mod source_registry;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

const DEFAULT_CONFIG_PATH: &str = "config/tsundoku.toml";

#[derive(Parser)]
#[command(
    name = "tsundoku",
    about = "Manga discovery service that polls sources and resolves releases to MangaBaka series",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the HTTP server
    Serve {
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
    },
    /// Run pending database migrations and exit
    Migrate {
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
    },
    /// Refresh the offline cache for every registered metadata provider
    /// (or one named provider). Providers without an offline cache are
    /// skipped silently. Distinct from `refresh-series`, which walks the
    /// existing series rows and re-fetches each one's metadata.
    RefreshProviderCache {
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Provider id to refresh; omit to iterate every registered provider.
        #[arg(short, long)]
        provider: Option<String>,
    },
    /// One-shot bulk refresh of stale series rows against the active
    /// metadata provider. Shares the same tick code as the scheduler
    /// cron and the `POST /api/v1/series/refresh-all` endpoint.
    RefreshSeries {
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Override `metadata.series_refresh.batch_size` for this run.
        #[arg(long)]
        batch_size: Option<u32>,
        /// Override `metadata.series_refresh.min_age_days` for this run.
        #[arg(long)]
        min_age_days: Option<u32>,
    },
    /// Run a one-shot poll against the configured discovery sources (or one
    /// named source). Releases are persisted as `unresolved`; the
    /// resolution pipeline runs separately.
    Poll {
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Source name to poll; omit to poll every enabled source.
        #[arg(short, long)]
        source: Option<String>,
    },
    /// Walk releases that have not been resolved yet and run them through
    /// the resolution pipeline. With `--retry-unresolved`, also re-run
    /// rows currently marked `ambiguous`. With `--include-resolved`,
    /// also re-evaluate rows currently marked `resolved` (skipping
    /// manually-linked ones) — use after changing format-type rules or
    /// title-cleanup config. Note: with `serve` running, prefer
    /// `POST /api/v1/releases/retry-all?includeResolved=true`, since
    /// running this CLI in parallel against the same SQLite file will
    /// contend for the write lock.
    Resolve {
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Also re-run rows currently marked `ambiguous` (e.g. after a
        /// provider refresh or a rules change).
        #[arg(long)]
        retry_unresolved: bool,
        /// Also re-evaluate rows currently marked `resolved`. Excludes
        /// manually-linked rows (resolution_path = 'manual') so a bulk
        /// retry doesn't overwrite operator decisions.
        #[arg(long)]
        include_resolved: bool,
    },
    /// One-shot historical catch-up for a single source. Walks the
    /// source's paginated HTML listing for `--pages` pages, persisting
    /// and resolving every new release. Idempotent on re-runs; does not
    /// affect the source's ETag / `last_polled_at` state.
    Backfill {
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Name of the discovery source to backfill. Must be a source
        /// whose kind opts in to `Backfillable` (currently `nyaa`).
        source: String,
        /// Number of listing pages to walk, starting at page 1.
        #[arg(short, long, default_value_t = 1)]
        pages: u32,
    },
    /// Write the OpenAPI specification to a file
    Openapi {
        #[arg(short, long, default_value = "web/openapi.json")]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve { config } => commands::serve::run(config).await,
        Commands::Migrate { config } => commands::migrate::run(config).await,
        Commands::RefreshProviderCache { config, provider } => {
            commands::refresh_provider_cache::run(config, provider).await
        }
        Commands::RefreshSeries {
            config,
            batch_size,
            min_age_days,
        } => commands::refresh_series::run(config, batch_size, min_age_days).await,
        Commands::Poll { config, source } => commands::poll::run(config, source).await,
        Commands::Resolve {
            config,
            retry_unresolved,
            include_resolved,
        } => commands::resolve::run(config, retry_unresolved, include_resolved).await,
        Commands::Backfill {
            config,
            source,
            pages,
        } => commands::backfill::run(config, source, pages).await,
        Commands::Openapi { output } => commands::openapi::run(&output),
    }
}

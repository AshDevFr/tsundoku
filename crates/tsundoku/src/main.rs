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
    /// skipped silently.
    RefreshMetadata {
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Provider id to refresh; omit to iterate every registered provider.
        #[arg(short, long)]
        provider: Option<String>,
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
    /// rows currently marked `ambiguous`.
    Resolve {
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Also re-run rows currently marked `ambiguous` (e.g. after a
        /// provider refresh or a rules change).
        #[arg(long)]
        retry_unresolved: bool,
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
        Commands::RefreshMetadata { config, provider } => {
            commands::refresh_metadata::run(config, provider).await
        }
        Commands::Poll { config, source } => commands::poll::run(config, source).await,
        Commands::Resolve {
            config,
            retry_unresolved,
        } => commands::resolve::run(config, retry_unresolved).await,
        Commands::Backfill {
            config,
            source,
            pages,
        } => commands::backfill::run(config, source, pages).await,
        Commands::Openapi { output } => commands::openapi::run(&output),
    }
}

mod api;
mod commands;
mod metadata;

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
        Commands::Openapi { output } => commands::openapi::run(&output),
    }
}

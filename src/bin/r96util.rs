use anyhow::Error;
use clap::{Parser, Subcommand};
use config::Config;
use log::{info, warn};
use route96::db::Database;
use route96::filesystem::FileStore;
use route96::settings::Settings;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[arg(long)]
    pub config: Option<String>,

    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Check file hash matches filename / path
    Check { delete: Option<bool> },

    /// Import a directory into the filesystem
    /// (does NOT import files into the database)
    Import { from: PathBuf },
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();

    let args: Args = Args::parse();

    let builder = Config::builder()
        .add_source(config::File::with_name(if let Some(ref c) = args.config {
            c.as_str()
        } else {
            "config.yaml"
        }))
        .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    let settings: Settings = builder.try_deserialize()?;

    match args.command {
        Commands::Check { delete } => {
            info!("Checking files in: {}", settings.storage_dir);
            let fs = FileStore::new(settings.clone());
            let mut dir = tokio::fs::read_dir(fs.storage_dir()).await?;
            while let Some(entry) = dir.next_entry().await? {
                if entry.file_type().await?.is_dir() {
                    continue;
                }

                let id = if let Ok(f) = hex::decode(entry.file_name().to_str().unwrap()) {
                    f
                } else {
                    warn!("Skipping invalid filename: {}", entry.path().display());
                    continue;
                };

                let hash = FileStore::hash_file(&entry.path()).await?;
                if hash != id {
                    if delete.unwrap_or(false) {
                        warn!("Deleting corrupt file: {}", entry.path().display());
                        tokio::fs::remove_file(entry.path()).await?;
                    } else {
                        warn!("File is corrupted: {}", entry.path().display());
                    }
                }
            }
        }
        Commands::Import { from } => {
            info!("Importing from: {}", from.display());
            let db = Database::new(&settings.database).await?;
        }
    }
    Ok(())
}

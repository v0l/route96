use anyhow::{Error, Result};
use clap::{Parser, Subcommand};
use config::Config;
use log::{debug, error, info, warn};
use route96::db::{Database, FileUpload};
use route96::filesystem::{FileStore, FileSystemResult};
use route96::settings::Settings;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::SystemTime;

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
    Check {
        #[arg(long)]
        delete: Option<bool>,
    },

    /// Import a directory into the filesystem
    /// (does NOT import files into the database, use database-import command for that)
    Import {
        #[arg(long)]
        from: PathBuf,
    },

    /// Import files from filesystem into database
    DatabaseImport {
        /// Don't actually import data and just print which files WOULD be imported
        #[arg(long, default_missing_value = "true", num_args = 0..=1)]
        dry_run: Option<bool>,
    },
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
            iter_files(&fs.storage_dir(), |entry| {
                Box::pin(async move {
                    let id = if let Some(i) = id_from_path(&entry) {
                        i
                    } else {
                        warn!("Skipping invalid file: {}", &entry.display());
                        return Ok(());
                    };

                    let hash = FileStore::hash_file(&entry).await?;
                    if hash != id {
                        if delete.unwrap_or(false) {
                            warn!("Deleting corrupt file: {}", &entry.display());
                            tokio::fs::remove_file(&entry).await?;
                        } else {
                            warn!("File is corrupted: {}", &entry.display());
                        }
                    }
                    Ok(())
                })
            })
            .await?;
        }
        Commands::Import { from } => {
            let fs = FileStore::new(settings.clone());
            let db = Database::new(&settings.database).await?;
            db.migrate().await?;
            info!("Importing from: {}", fs.storage_dir().display());
            iter_files(&from, |entry| {
                let fs = fs.clone();
                Box::pin(async move {
                    let mime = infer::get_from_path(&entry)?
                        .map(|m| m.mime_type())
                        .unwrap_or("application/octet-stream");
                    let file = tokio::fs::File::open(&entry).await?;
                    let dst = fs.put(file, mime, false).await?;
                    match dst {
                        FileSystemResult::AlreadyExists(_) => {
                            info!("Duplicate file: {}", &entry.display())
                        }
                        FileSystemResult::NewFile(_) => info!("Imported: {}", &entry.display()),
                    }
                    Ok(())
                })
            })
            .await?;
        }
        Commands::DatabaseImport { dry_run } => {
            let fs = FileStore::new(settings.clone());
            let db = Database::new(&settings.database).await?;
            db.migrate().await?;
            info!("Importing to DB from: {}", fs.storage_dir().display());
            iter_files(&fs.storage_dir(), |entry| {
                let db = db.clone();
                Box::pin(async move {
                    let id = if let Some(i) = id_from_path(&entry) {
                        i
                    } else {
                        warn!("Skipping invalid file: {}", &entry.display());
                        return Ok(());
                    };
                    let u = db.get_file(&id).await?;
                    if u.is_none() {
                        if !dry_run.unwrap_or(false) {
                            info!("Importing file: {}", &entry.display());
                            let mime = infer::get_from_path(&entry)?
                                .map(|m| m.mime_type())
                                .unwrap_or("application/octet-stream")
                                .to_string();
                            let entry = FileUpload {
                                id,
                                name: None,
                                size: entry.metadata()?.len(),
                                mime_type: mime,
                                created: entry
                                    .metadata()?
                                    .created()
                                    .unwrap_or(SystemTime::now())
                                    .into(),
                                width: None,
                                height: None,
                                blur_hash: None,
                                alt: None,
                                duration: None,
                                bitrate: None,
                            };
                            db.add_file(&entry, None).await?;
                        } else {
                            info!("[DRY-RUN] Importing file: {}", &entry.display());
                        }
                    }
                    Ok(())
                })
            })
            .await?;
        }
    }
    Ok(())
}

fn id_from_path(path: &Path) -> Option<Vec<u8>> {
    hex::decode(path.file_name()?.to_str()?).ok()
}

async fn iter_files<F>(p: &Path, mut op: F) -> Result<()>
where
    F: FnMut(PathBuf) -> Pin<Box<dyn Future<Output = Result<()>>>>,
{
    info!("Scanning files: {}", p.display());
    let entries = walkdir::WalkDir::new(p);
    for entry in entries
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        debug!("Checking file: {}", entry.path().display());
        if let Err(e) = op(entry.path().to_path_buf()).await {
            error!("Error processing file: {} {}", entry.path().display(), e);
        }
    }
    Ok(())
}

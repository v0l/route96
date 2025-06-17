use anyhow::{Context, Error, Result};
use clap::{Parser, Subcommand};
use config::Config;
use indicatif::{ProgressBar, ProgressStyle};
use log::{error, info};
use route96::db::{Database, FileUpload};
use route96::filesystem::{FileStore, FileSystemResult};
use route96::processing::probe_file;
use route96::settings::Settings;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;
use pretty_env_logger::env_logger;
use tokio::sync::Semaphore;

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
        #[arg(long, default_missing_value = "true", num_args = 0..=1)]
        probe_media: Option<bool>,
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
        unsafe { std::env::set_var("RUST_LOG", "info"); }
    }
    env_logger::init();

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
            iter_files(&fs.storage_dir(), 4, |entry, p| {
                let p = p.clone();
                Box::pin(async move {
                    let id = if let Some(i) = id_from_path(&entry) {
                        i
                    } else {
                        p.set_message(format!("Skipping invalid file: {}", &entry.display()));
                        return Ok(());
                    };

                    let hash = FileStore::hash_file(&entry).await?;
                    if hash != id {
                        if delete.unwrap_or(false) {
                            p.set_message(format!("Deleting corrupt file: {}", &entry.display()));
                            tokio::fs::remove_file(&entry).await?;
                        } else {
                            p.set_message(format!("File is corrupted: {}", &entry.display()));
                        }
                    }
                    Ok(())
                })
            })
                .await?;
        }
        Commands::Import { from, probe_media } => {
            let fs = FileStore::new(settings.clone());
            let db = Database::new(&settings.database).await?;
            db.migrate().await?;
            info!("Importing from: {}", fs.storage_dir().display());
            iter_files(&from, 4, |entry, p| {
                let fs = fs.clone();
                let p = p.clone();
                Box::pin(async move {
                    let mime = infer::get_from_path(&entry)?
                        .map(|m| m.mime_type())
                        .unwrap_or("application/octet-stream");

                    // test media is not corrupt
                    if probe_media.unwrap_or(true)
                        && (mime.starts_with("image/") || mime.starts_with("video/"))
                        && probe_file(&entry).is_err()
                    {
                        p.set_message(format!("Skipping media invalid file: {}", &entry.display()));
                        return Ok(());
                    }

                    let file = tokio::fs::File::open(&entry).await?;
                    let dst = fs.put(file, mime, false).await?;
                    match dst {
                        FileSystemResult::AlreadyExists(_) => {
                            p.set_message(format!("Duplicate file: {}", &entry.display()));
                        }
                        FileSystemResult::NewFile(_) => {
                            p.set_message(format!("Imported: {}", &entry.display()));
                        }
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
            iter_files(&fs.storage_dir(), 4, |entry, p| {
                let db = db.clone();
                let p = p.clone();
                Box::pin(async move {
                    let id = if let Some(i) = id_from_path(&entry) {
                        i
                    } else {
                        p.set_message(format!("Skipping invalid file: {}", &entry.display()));
                        return Ok(());
                    };
                    let u = db.get_file(&id).await.context("db get_file")?;
                    if u.is_none() {
                        if !dry_run.unwrap_or(false) {
                            p.set_message(format!("Importing file: {}", &entry.display()));
                            let mime = infer::get_from_path(&entry)
                                .context("infer")?
                                .map(|m| m.mime_type())
                                .unwrap_or("application/octet-stream")
                                .to_string();
                            let meta = entry.metadata().context("file metadata")?;
                            let entry = FileUpload {
                                id,
                                name: None,
                                size: meta.len(),
                                mime_type: mime,
                                created: meta.created().unwrap_or(SystemTime::now()).into(),
                                width: None,
                                height: None,
                                blur_hash: None,
                                alt: None,
                                duration: None,
                                bitrate: None,
                            };
                            db.add_file(&entry, None).await.context("db add_file")?;
                        } else {
                            p.set_message(format!(
                                "[DRY-RUN] Importing file: {}",
                                &entry.display()
                            ));
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

async fn iter_files<F>(p: &Path, threads: usize, mut op: F) -> Result<()>
where
    F: FnMut(PathBuf, ProgressBar) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>,
{
    let semaphore = Arc::new(Semaphore::new(threads));
    info!("Scanning files: {}", p.display());
    let entries = walkdir::WalkDir::new(p);
    let dir = entries
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .collect::<Vec<_>>();
    let p = ProgressBar::new(dir.len() as u64).with_style(ProgressStyle::with_template(
        "{spinner} [{pos}/{len}] {msg}",
    )?);
    let mut all_tasks = vec![];
    for entry in dir {
        let _lock = semaphore.clone().acquire_owned().await?;
        p.inc(1);
        let fut = op(entry.path().to_path_buf(), p.clone());
        all_tasks.push(tokio::spawn(async move {
            if let Err(e) = fut.await {
                error!("Error processing file: {} {}", entry.path().display(), e);
            }
            drop(_lock);
        }));
    }
    for task in all_tasks {
        task.await?;
    }
    p.finish_with_message("Done!");
    Ok(())
}
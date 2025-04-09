use anyhow::{anyhow, Context, Error, Result};
use clap::{Parser, Subcommand};
use config::Config;
use indicatif::{ProgressBar, ProgressStyle};
use log::{error, info, warn};
use route96::db::{Database, FileUpload};
use route96::filesystem::FileStore;
use route96::processing::probe_file;
use route96::settings::{Settings, StorageBackendType};
#[cfg(feature = "s3-storage")]
use route96::spaces::SpacesStore;
use route96::storage::{StorageBackend, StorageResult};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt};
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
    /// Verify file hash matches filename / path and optionally delete mismatches.
    VerifyIntegrity {
        #[arg(long)]
        delete: Option<bool>,
    },

    /// Import files from an external directory into the storage directory.
    /// Does NOT index files into the database; use index-storage for that.
    ImportFiles {
        #[arg(long)]
        from: PathBuf,
        #[arg(long, default_missing_value = "true", num_args = 0..=1)]
        probe_media: Option<bool>,
    },

    /// Scan storage directory and add files missing from the database index.
    IndexStorage {
        /// Print files that would be indexed without actually modifying the database.
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

    let settings: Settings = builder
        .try_deserialize()
        .context("Failed to deserialize settings")?;

    let storage: Arc<dyn StorageBackend> = match settings.storage_type {
        StorageBackendType::FileSystem => {
            info!("Using FileSystem storage backend.");
            let fs_settings = settings.filesystem.clone().context(
                "FileSystem storage type selected, but [filesystem] settings are missing",
            )?;
            let temp_dir = PathBuf::from(&settings.temp_dir);
            tokio::fs::create_dir_all(&temp_dir)
                .await
                .context(format!("Failed to create temp directory: {:?}", temp_dir))?;

            let fs = FileStore::new(&fs_settings, temp_dir);
            Arc::new(fs)
        }
        #[cfg(feature = "s3-storage")]
        StorageBackendType::S3 => {
            info!("Using S3 storage backend.");
            let s3_settings = settings
                .s3
                .clone()
                .context("S3 storage type selected, but [s3] settings are missing")?;
            let temp_dir = PathBuf::from(&settings.temp_dir);
            tokio::fs::create_dir_all(&temp_dir)
                .await
                .context(format!("Failed to create temp directory: {:?}", temp_dir))?;

            let spaces = SpacesStore::new(&s3_settings, temp_dir)
                .await
                .context("Failed to initialize S3 storage client")?;
            Arc::new(spaces)
        }
    };

    match args.command {
        Commands::VerifyIntegrity { delete } => {
            if settings.storage_type != StorageBackendType::FileSystem {
                return Err(anyhow!(
                    "'VerifyIntegrity' command only supports the FileSystem storage backend."
                ));
            }

            let fs_settings = settings
                .filesystem
                .context("[filesystem] settings missing")?;
            let storage_dir = PathBuf::from(&fs_settings.storage_dir);
            info!("Checking files in: {}", storage_dir.display());

            iter_files(&storage_dir, 4, |entry, p| {
                let p = p.clone();
                Box::pin(async move {
                    let id = if let Some(i) = id_from_path(&entry) {
                        i
                    } else {
                        p.set_message(format!("Skipping invalid path: {}", &entry.display()));
                        return Ok(());
                    };

                    let calculated_hash = match hash_local_file(&entry).await {
                        Ok(h) => h,
                        Err(e) => {
                            p.set_message(format!("Error hashing {}: {}", entry.display(), e));
                            return Ok(());
                        }
                    };

                    if calculated_hash != id {
                        if delete.unwrap_or(false) {
                            p.set_message(format!("Deleting corrupt file: {}", &entry.display()));
                            if let Err(e) = tokio::fs::remove_file(&entry).await {
                                p.set_message(format!(
                                    "Failed to delete {}: {}",
                                    entry.display(),
                                    e
                                ));
                            }
                        } else {
                            p.set_message(format!("File is corrupted: {}", &entry.display()));
                        }
                    } else {
                        p.set_message(format!("Verified OK: {}", &entry.display()));
                    }
                    Ok(())
                })
            })
            .await?;
        }
        Commands::ImportFiles { from, probe_media } => {
            let db = Database::new(&settings.database).await?;
            db.migrate().await?;
            info!("Importing from directory: {}", from.display());

            iter_files(&from, 4, |entry, p| {
                let storage = storage.clone();
                let p = p.clone();
                Box::pin(async move {
                    let mime = infer::get_from_path(&entry)?
                        .map(|m| m.mime_type())
                        .unwrap_or("application/octet-stream");

                    if probe_media.unwrap_or(true)
                        && (mime.starts_with("image/") || mime.starts_with("video/"))
                        && probe_file(&entry).is_err()
                    {
                        p.set_message(format!("Skipping invalid media file: {}", &entry.display()));
                        return Ok(());
                    }

                    let file = File::open(&entry)
                        .await
                        .context("Failed to open file for import")?;
                    let boxed_stream: Box<dyn AsyncRead + Send + Unpin> = Box::pin(file);

                    let dst = storage
                        .put(boxed_stream, mime)
                        .await
                        .context("Storage put failed")?;

                    match dst {
                        StorageResult::AlreadyExists(hash) => {
                            p.set_message(format!(
                                "Duplicate file (hash: {}): {}",
                                hex::encode(&hash),
                                &entry.display()
                            ));
                        }
                        StorageResult::NewFile { id, .. } => {
                            p.set_message(format!(
                                "Imported (hash: {}): {}",
                                hex::encode(&id),
                                &entry.display()
                            ));
                        }
                    }
                    Ok(())
                })
            })
            .await?;
        }
        Commands::IndexStorage { dry_run } => {
            if settings.storage_type != StorageBackendType::FileSystem {
                return Err(anyhow!(
                    "'IndexStorage' command only supports the FileSystem storage backend."
                ));
            }
            let fs_settings = settings
                .filesystem
                .context("[filesystem] settings missing")?;
            let storage_dir = PathBuf::from(&fs_settings.storage_dir);

            let db = Database::new(&settings.database).await?;
            db.migrate().await?;
            info!(
                "Indexing DB from storage directory: {}",
                storage_dir.display()
            );

            iter_files(&storage_dir, 4, |entry, p| {
                let db = db.clone();
                let p = p.clone();
                Box::pin(async move {
                    let id = if let Some(i) = id_from_path(&entry) {
                        i
                    } else {
                        p.set_message(format!("Skipping invalid path: {}", &entry.display()));
                        return Ok(());
                    };

                    let u = db.get_file(&id).await.context("db get_file")?;
                    if u.is_none() {
                        if !dry_run.unwrap_or(false) {
                            p.set_message(format!("Indexing file: {}", &entry.display()));

                            let mime = infer::get_from_path(&entry)
                                .context("infer mime type")?
                                .map(|m| m.mime_type())
                                .unwrap_or("application/octet-stream")
                                .to_string();
                            let meta = entry.metadata().context("get file metadata")?;

                            let upload_entry = FileUpload {
                                id,
                                size: meta.len() as i64,
                                mime_type: mime,
                                created: meta.created().unwrap_or(SystemTime::now()).into(),
                                width: None,
                                height: None,
                                blur_hash: None,
                                alt: None,
                                duration: None,
                                bitrate: None,
                                h_tag: None,
                                #[cfg(feature = "labels")]
                                labels: vec![],
                            };

                            db.add_file(&upload_entry, None)
                                .await
                                .context("db add_file")?;
                        } else {
                            p.set_message(format!(
                                "[DRY-RUN] Would index file: {}",
                                &entry.display()
                            ));
                        }
                    } else {
                        p.set_message(format!("Already indexed: {}", &entry.display()));
                    }
                    Ok(())
                })
            })
            .await?;
        }
    }
    Ok(())
}

async fn hash_local_file(p: &Path) -> Result<Vec<u8>, Error> {
    let mut file = File::open(p)
        .await
        .context(format!("Failed to open file for hashing: {:?}", p))?;
    let mut hasher = Sha256::new();
    let mut buf = [0; 8192];

    loop {
        let n = file
            .read(&mut buf)
            .await
            .context(format!("Failed to read file for hashing: {:?}", p))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let res = hasher.finalize();
    Ok(res.to_vec())
}

async fn iter_files<F>(dir: &PathBuf, max_concurrent: usize, func: F) -> Result<()>
where
    F: Fn(PathBuf, Arc<ProgressBar>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>
        + Send
        + Sync
        + 'static,
{
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut entries = tokio::fs::read_dir(dir).await?;
    let mut tasks = vec![];
    let pb = Arc::new(ProgressBar::new_spinner());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] {wide_msg}")
            .unwrap()
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let func = Arc::new(func);
            let pb_clone = pb.clone();
            tasks.push(tokio::spawn(async move {
                let result = func(path, pb_clone).await;
                drop(permit);
                result
            }));
        }
    }

    let mut results = vec![];
    for task in tasks {
        results.push(task.await);
    }

    for result in results {
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(anyhow!(e)),
        }
    }

    pb.finish_with_message("Done.");
    Ok(())
}

fn id_from_path(path: &Path) -> Option<Vec<u8>> {
    hex::decode(path.file_name()?.to_str()?).ok()
}

//! Background task: compute perceptual hashes for images that are missing them.

use crate::db::Database;
use crate::filesystem::FileStore;
use crate::phash::phash_image;
use log::{error, info, warn};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct PhashFiles {
    db: Database,
    fs: FileStore,
}

impl PhashFiles {
    pub fn new(db: Database, fs: FileStore) -> Self {
        Self { db, fs }
    }

    /// Run the background phash-computation loop.
    ///
    /// Processes up to 100 images per cycle, sleeping 2 s between cycles.
    /// Shuts down cleanly when `shutdown` is cancelled.
    pub async fn process(self, shutdown: CancellationToken) {
        let db = self.db.clone();
        let fs = self.fs.clone();

        tokio::spawn(async move {
            info!("PhashFiles worker started");
            loop {
                tokio::select! {
                    _ = Self::run_batch(&db, &fs) => {}
                    _ = shutdown.cancelled() => {
                        info!("PhashFiles worker shutting down");
                        return;
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|e| error!("PhashFiles task failed: {:?}", e));
    }

    async fn run_batch(db: &Database, fs: &FileStore) {
        let to_process = match db.get_images_missing_phash().await {
            Ok(v) => v,
            Err(e) => {
                error!("PhashFiles: failed to query missing phashes: {}", e);
                tokio::time::sleep(Duration::from_secs(2)).await;
                return;
            }
        };

        if !to_process.is_empty() {
            info!("{} images missing phash", to_process.len());
        }

        for file in to_process {
            let path = fs.get(&file.id);
            if !path.exists() {
                warn!(
                    "PhashFiles: skipping missing file {}",
                    hex::encode(&file.id)
                );
                if let Err(e) = db.upsert_phash(&file.id, &[0u8; 8]).await {
                    error!(
                        "PhashFiles: failed to mark missing file {}: {}",
                        hex::encode(&file.id),
                        e
                    );
                }
                continue;
            }

            let hash = match tokio::task::spawn_blocking(move || {
                phash_image(&path, &file.mime_type)
            })
            .await
            {
                Ok(Ok(hash)) => hash,
                Ok(Err(e)) => {
                    warn!(
                        "PhashFiles: failed to hash {}: {}",
                        hex::encode(&file.id),
                        e
                    );
                    if let Err(e) = db.upsert_phash(&file.id, &[0u8; 8]).await {
                        error!(
                            "PhashFiles: failed to mark broken file {}: {}",
                            hex::encode(&file.id),
                            e
                        );
                    }
                    continue;
                }
                Err(e) => {
                    error!(
                        "PhashFiles: task panicked for {}: {}",
                        hex::encode(&file.id),
                        e
                    );
                    if let Err(e) = db.upsert_phash(&file.id, &[0u8; 8]).await {
                        error!(
                            "PhashFiles: failed to mark file after panic {}: {}",
                            hex::encode(&file.id),
                            e
                        );
                    }
                    continue;
                }
            };

            let bytes: &[u8; 8] = hash.as_bytes().try_into().unwrap();
            if let Err(e) = db.upsert_phash(&file.id, bytes).await {
                error!(
                    "PhashFiles: failed to store phash for {}: {}",
                    hex::encode(&file.id),
                    e
                );
            } else {
                info!(
                    "PhashFiles: computed phash {} for {}",
                    hex::encode(bytes),
                    hex::encode(&file.id)
                );
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

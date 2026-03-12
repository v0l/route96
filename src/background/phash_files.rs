//! Background task: compute perceptual hashes for images that are missing them.

use super::{BatchResult, next_sleep};
use crate::db::Database;
use crate::filesystem::FileStore;
use crate::phash::phash_image;
use log::{error, info, warn};
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
    /// Processes up to 100 images per cycle, sleeping between cycles.
    /// Backs off exponentially when the same files keep reappearing.
    /// Shuts down cleanly when `shutdown` is cancelled.
    pub async fn process(self, shutdown: CancellationToken) {
        let db = self.db.clone();
        let fs = self.fs.clone();

        tokio::spawn(async move {
            info!("PhashFiles worker started");

            let mut prev_count: usize = 0;
            let mut stall_rounds: u32 = 0;

            loop {
                let sleep_dur;

                tokio::select! {
                    batch_result = Self::run_batch(&db, &fs, &shutdown) => {
                        sleep_dur = next_sleep(&batch_result, &mut prev_count, &mut stall_rounds);
                        if let BatchResult::Processed { found } = batch_result
                            && stall_rounds > 0
                        {
                            warn!(
                                "PhashFiles: stalled on {} files, backing off {:.0?}",
                                found,
                                sleep_dur,
                            );
                        }
                    }
                    _ = shutdown.cancelled() => {
                        info!("PhashFiles worker shutting down");
                        return;
                    }
                }

                // Sleep outside the select so the cancellation token can
                // still interrupt us during the wait.
                tokio::select! {
                    _ = tokio::time::sleep(sleep_dur) => {}
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

    async fn run_batch(db: &Database, fs: &FileStore, shutdown: &CancellationToken) -> BatchResult {
        let to_process = match db.get_images_missing_phash().await {
            Ok(v) => v,
            Err(e) => {
                error!("PhashFiles: failed to query missing phashes: {}", e);
                return BatchResult::Idle;
            }
        };

        if to_process.is_empty() {
            return BatchResult::Idle;
        }

        let found = to_process.len();
        info!("{} images missing phash", found);

        for file in to_process {
            if shutdown.is_cancelled() {
                return BatchResult::Processed { found };
            }
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

        BatchResult::Processed { found }
    }
}

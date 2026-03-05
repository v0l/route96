//! Background task: compute perceptual hashes for images that are missing them.

use crate::db::Database;
use crate::filesystem::FileStore;
use crate::phash::phash_image;
use log::{error, info, warn};
use std::time::Duration;
use tokio::runtime::Handle;
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
        let handle = Handle::current();

        let db = self.db.clone();
        let fs = self.fs.clone();

        tokio::task::spawn_blocking(move || {
            info!("PhashFiles worker started");
            loop {
                Self::run_batch(&db, &fs, &handle);

                if shutdown.is_cancelled() {
                    info!("PhashFiles worker shutting down");
                    return;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .await
        .unwrap_or_else(|e| error!("PhashFiles spawn_blocking failed: {:?}", e));
    }

    fn run_batch(db: &Database, fs: &FileStore, handle: &Handle) {
        let to_process = match handle.block_on(db.get_images_missing_phash()) {
            Ok(v) => v,
            Err(e) => {
                error!("PhashFiles: failed to query missing phashes: {}", e);
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
                // Insert a dummy sentinel so we don't keep retrying.
                // Re-use upsert_phash with hash=0; Hamming distance to any
                // real image will be large and it won't surface in results.
                if let Err(e) = handle.block_on(db.upsert_phash(&file.id, &[0u8; 8])) {
                    error!(
                        "PhashFiles: failed to mark missing file {}: {}",
                        hex::encode(&file.id),
                        e
                    );
                }
                continue;
            }

            match phash_image(&path, &file.mime_type) {
                Ok(hash) => {
                    let bytes: &[u8; 8] = hash.as_bytes().try_into().unwrap();
                    if let Err(e) = handle.block_on(db.upsert_phash(&file.id, bytes)) {
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
                Err(e) => {
                    warn!(
                        "PhashFiles: failed to hash {}: {}",
                        hex::encode(&file.id),
                        e
                    );
                    // Store zeroed hash so we don't retry broken files forever.
                    let _ = handle.block_on(db.upsert_phash(&file.id, &[0u8; 8]));
                }
            }
        }
    }
}

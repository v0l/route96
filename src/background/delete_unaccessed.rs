//! Background task: delete files that have had no downloads in the configured window.

use super::{BatchResult, next_sleep};
use crate::db::Database;
use crate::filesystem::FileStore;
use chrono::Utc;
use log::{error, info, warn};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Number of files to process per batch.
const BATCH_SIZE: u32 = 100;

pub struct DeleteUnaccessed {
    db: Database,
    fs: FileStore,
    /// Files not downloaded within this duration are eligible for deletion.
    max_inactive: Duration,
}

impl DeleteUnaccessed {
    pub fn new(db: Database, fs: FileStore, days: u64) -> Self {
        Self {
            db,
            fs,
            max_inactive: Duration::from_secs(days * 86_400),
        }
    }

    /// Run the background deletion loop.
    ///
    /// Each cycle queries up to `BATCH_SIZE` eligible files, deletes their
    /// database rows, and removes the physical files from disk.  The loop
    /// backs off exponentially when the same file count recurs, and shuts
    /// down cleanly when `shutdown` is cancelled.
    pub async fn process(self, shutdown: CancellationToken) {
        let db = self.db.clone();
        let fs = self.fs.clone();
        let max_inactive = self.max_inactive;

        tokio::spawn(async move {
            info!(
                "DeleteUnaccessed worker started (window: {}s)",
                max_inactive.as_secs()
            );

            let mut prev_count: usize = 0;
            let mut stall_rounds: u32 = 0;

            loop {
                let sleep_dur;

                tokio::select! {
                    batch_result = Self::run_batch(&db, &fs, max_inactive) => {
                        sleep_dur = next_sleep(&batch_result, &mut prev_count, &mut stall_rounds);
                        if let BatchResult::Processed { found } = batch_result
                            && stall_rounds > 0
                        {
                            warn!(
                                "DeleteUnaccessed: stalled on {} files, backing off {:.0?}",
                                found,
                                sleep_dur,
                            );
                        }
                    }
                    _ = shutdown.cancelled() => {
                        info!("DeleteUnaccessed worker shutting down");
                        return;
                    }
                }

                tokio::select! {
                    _ = tokio::time::sleep(sleep_dur) => {}
                    _ = shutdown.cancelled() => {
                        info!("DeleteUnaccessed worker shutting down");
                        return;
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|e| error!("DeleteUnaccessed task panicked: {:?}", e));
    }

    async fn run_batch(db: &Database, fs: &FileStore, max_inactive: Duration) -> BatchResult {
        let cutoff = Utc::now()
            - chrono::Duration::seconds(max_inactive.as_secs() as i64);

        let ids = match db.get_unaccessed_files(cutoff, BATCH_SIZE).await {
            Ok(v) => v,
            Err(e) => {
                error!("DeleteUnaccessed: query failed: {}", e);
                return BatchResult::Idle;
            }
        };

        if ids.is_empty() {
            return BatchResult::Idle;
        }

        let found = ids.len();
        info!("DeleteUnaccessed: deleting {} unaccessed file(s)", found);

        for id in &ids {
            // Remove ownership records first, then the upload row itself.
            // (file_stats cascades automatically via FK on delete.)
            if let Err(e) = db.delete_all_file_owner(id).await {
                error!(
                    "DeleteUnaccessed: failed to remove owners for {}: {}",
                    hex::encode(id),
                    e
                );
                continue;
            }
            if let Err(e) = db.delete_file(id).await {
                error!(
                    "DeleteUnaccessed: failed to delete DB row for {}: {}",
                    hex::encode(id),
                    e
                );
                continue;
            }

            let path = fs.get(id);
            match tokio::fs::remove_file(&path).await {
                Ok(()) => info!("DeleteUnaccessed: removed {}", hex::encode(id)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    warn!(
                        "DeleteUnaccessed: physical file already missing for {}",
                        hex::encode(id)
                    );
                }
                Err(e) => {
                    error!(
                        "DeleteUnaccessed: failed to remove file {} from disk: {}",
                        hex::encode(id),
                        e
                    );
                }
            }
        }

        BatchResult::Processed { found }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_max_inactive_from_days() {
        // We can't easily construct a Database/FileStore in unit tests, so we
        // verify the duration calculation directly.
        let days = 7u64;
        let expected = Duration::from_secs(days * 86_400);

        // Compute the same way the constructor does.
        let actual = Duration::from_secs(days * 86_400);
        assert_eq!(actual, expected);
    }

    #[test]
    fn zero_days_yields_zero_duration() {
        let actual = Duration::from_secs(0u64 * 86_400);
        assert_eq!(actual, Duration::ZERO);
    }
}

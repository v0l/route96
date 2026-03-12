//! Background task: enforce file-retention policies.
//!
//! Two independent policies can be active simultaneously:
//!
//! * **Inactivity** (`delete_unaccessed_days`): delete files that have had no
//!   downloads within the configured window.  Files uploaded within the same
//!   window receive an implicit grace period.
//! * **Hard age** (`delete_after_days`): delete all files older than the
//!   configured duration, regardless of download activity.
//!
//! Thresholds are re-read from the live settings on every cycle so that
//! config changes take effect without a restart.

use super::{BatchResult, next_sleep};
use crate::db::Database;
use crate::filesystem::FileStore;
use crate::settings::Settings;
use chrono::Utc;
use log::{error, info, warn};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Number of files to process per batch per policy.
const BATCH_SIZE: u32 = 100;

pub struct FileDeleter {
    db: Database,
    fs: FileStore,
    settings: Arc<RwLock<Settings>>,
}

impl FileDeleter {
    pub fn new(db: Database, fs: FileStore, settings: Arc<RwLock<Settings>>) -> Self {
        Self { db, fs, settings }
    }

    /// Run the background deletion loop.
    ///
    /// Thresholds are re-read from `settings` on every cycle so runtime
    /// config changes take effect immediately.  The loop shuts down cleanly
    /// when `shutdown` is cancelled.
    pub async fn process(self, shutdown: CancellationToken) {
        tokio::spawn(async move {
            info!("FileDeleter worker started");

            let mut prev_count: usize = 0;
            let mut stall_rounds: u32 = 0;

            loop {
                let sleep_dur;

                tokio::select! {
                    batch_result = Self::run_batch(&self.db, &self.fs, &self.settings) => {
                        sleep_dur = next_sleep(&batch_result, &mut prev_count, &mut stall_rounds);
                        if let BatchResult::Processed { found } = batch_result
                            && stall_rounds > 0
                        {
                            warn!(
                                "FileDeleter: stalled on {} files, backing off {:.0?}",
                                found,
                                sleep_dur,
                            );
                        }
                    }
                    _ = shutdown.cancelled() => {
                        info!("FileDeleter worker shutting down");
                        return;
                    }
                }

                tokio::select! {
                    _ = tokio::time::sleep(sleep_dur) => {}
                    _ = shutdown.cancelled() => {
                        info!("FileDeleter worker shutting down");
                        return;
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|e| error!("FileDeleter task panicked: {:?}", e));
    }

    async fn run_batch(
        db: &Database,
        fs: &FileStore,
        settings: &Arc<RwLock<Settings>>,
    ) -> BatchResult {
        // Snapshot thresholds from live config for this cycle.
        let (inactive_days, age_days) = {
            let s = settings.read().await;
            (s.delete_unaccessed_days, s.delete_after_days)
        };

        let now = Utc::now();
        // Each entry is (file_id, reason) so the reason is logged on deletion.
        let mut ids: Vec<(Vec<u8>, &'static str)> = Vec::new();

        // Inactivity policy.
        if let Some(days) = inactive_days.filter(|&d| d > 0) {
            let cutoff = now - chrono::Duration::seconds((days * 86_400) as i64);
            match db.get_unaccessed_files(cutoff, BATCH_SIZE).await {
                Ok(v) => ids.extend(v.into_iter().map(|id| (id, "inactive"))),
                Err(e) => error!("FileDeleter: inactivity query failed: {}", e),
            }
        }

        // Hard-age policy.
        if let Some(days) = age_days.filter(|&d| d > 0) {
            let cutoff = now - chrono::Duration::seconds((days * 86_400) as i64);
            match db.get_files_older_than(cutoff, BATCH_SIZE).await {
                Ok(v) => {
                    for id in v {
                        if !ids.iter().any(|(existing, _)| existing == &id) {
                            ids.push((id, "expired"));
                        }
                    }
                }
                Err(e) => error!("FileDeleter: hard-age query failed: {}", e),
            }
        }

        if ids.is_empty() {
            return BatchResult::Idle;
        }

        let found = ids.len();
        info!("FileDeleter: deleting {} file(s)", found);

        for (id, reason) in &ids {
            Self::delete_one(db, fs, id, reason).await;
        }

        BatchResult::Processed { found }
    }

    async fn delete_one(db: &Database, fs: &FileStore, id: &Vec<u8>, reason: &str) {
        // Remove ownership records first, then the upload row.
        // file_stats cascades automatically via FK on delete.
        if let Err(e) = db.delete_all_file_owner(id).await {
            error!(
                "FileDeleter: failed to remove owners for {}: {}",
                hex::encode(id),
                e
            );
            return;
        }
        if let Err(e) = db.delete_file(id).await {
            error!(
                "FileDeleter: failed to delete DB row for {}: {}",
                hex::encode(id),
                e
            );
            return;
        }

        let path = fs.get(id);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => info!("FileDeleter: removed {} ({})", hex::encode(id), reason),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                warn!(
                    "FileDeleter: physical file already missing for {} ({})",
                    hex::encode(id),
                    reason
                );
            }
            Err(e) => {
                error!(
                    "FileDeleter: failed to remove {} from disk ({}): {}",
                    hex::encode(id),
                    reason,
                    e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    fn days_to_secs(d: u64) -> u64 {
        d * 86_400
    }

    #[test]
    fn days_to_secs_correct() {
        assert_eq!(days_to_secs(0), 0);
        assert_eq!(days_to_secs(1), 86_400);
        assert_eq!(days_to_secs(7), 604_800);
        assert_eq!(days_to_secs(30), 2_592_000);
    }

    #[test]
    fn zero_days_filter_disables_policy() {
        // Mirrors the `.filter(|&d| d > 0)` guard used in run_batch.
        assert!(Some(0u64).filter(|&d| d > 0).is_none());
        assert!(Some(1u64).filter(|&d| d > 0).is_some());
    }

    #[test]
    fn none_days_disables_policy() {
        assert!(None::<u64>.filter(|&d| d > 0).is_none());
    }

    #[test]
    fn duration_from_days_matches_seconds() {
        let d = 30u64;
        assert_eq!(
            Duration::from_secs(days_to_secs(d)),
            Duration::from_secs(30 * 86_400)
        );
    }
}

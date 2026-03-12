use crate::db::Database;
use crate::file_stats::FileStatsTracker;
use crate::filesystem::FileStore;
use crate::settings::Settings;
use log::{error, info};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// How long to wait between batches when there is no work.
const IDLE_SLEEP: Duration = Duration::from_secs(30);

/// How long to wait between batches when the previous batch processed files.
const BATCH_SLEEP: Duration = Duration::from_secs(2);

/// Upper limit for back-off when the same files keep reappearing.
const MAX_BACKOFF: Duration = Duration::from_secs(300);

/// Outcome of a single batch-processing invocation used by background workers.
enum BatchResult {
    /// The query returned zero files — nothing to do.
    Idle,
    /// The batch found (and attempted to process) `found` files.
    Processed { found: usize },
}

/// Decide how long to sleep after a batch, updating stall-tracking state.
///
/// Returns the recommended sleep duration and updates `prev_count` /
/// `stall_rounds` in place.
fn next_sleep(result: &BatchResult, prev_count: &mut usize, stall_rounds: &mut u32) -> Duration {
    match result {
        BatchResult::Idle => {
            *stall_rounds = 0;
            *prev_count = 0;
            IDLE_SLEEP
        }
        BatchResult::Processed { found } => {
            let found = *found;
            let dur = if found > 0 && found == *prev_count {
                *stall_rounds = stall_rounds.saturating_add(1);
                BATCH_SLEEP
                    .saturating_mul(1u32 << (*stall_rounds).min(10))
                    .min(MAX_BACKOFF)
            } else {
                *stall_rounds = 0;
                BATCH_SLEEP
            };
            *prev_count = found;
            dur
        }
    }
}

mod file_deleter;

#[cfg(feature = "media-compression")]
mod media_metadata;

#[cfg(feature = "media-compression")]
mod phash_files;

#[cfg(feature = "labels")]
mod label_files;

#[cfg(feature = "payments")]
mod payments;

pub async fn start_background_tasks(
    db: Database,
    file_store: FileStore,
    settings: Arc<RwLock<Settings>>,
    shutdown: CancellationToken,
    file_stats: FileStatsTracker,
    #[cfg(feature = "payments")] client: Option<fedimint_tonic_lnd::Client>,
) -> JoinSet<()> {
    #[allow(unused_variables)]
    let settings_snap = settings.read().await.clone();

    let mut set = JoinSet::new();

    // Always start the file-stats flush task.
    set.spawn(file_stats.flush_task(db.clone(), shutdown.clone()));

    #[cfg(feature = "media-compression")]
    {
        let db = db.clone();
        let fs = file_store.clone();
        let token = shutdown.clone();
        set.spawn(async move {
            info!("Starting MediaMetadata background task");
            let mut m = media_metadata::MediaMetadata::new(db, fs);
            if let Err(e) = m.process(token).await {
                error!("MediaMetadata failed: {}", e);
            } else {
                info!("MediaMetadata background task completed");
            }
        });
    }

    #[cfg(feature = "media-compression")]
    {
        let db = db.clone();
        let fs = file_store.clone();
        let token = shutdown.clone();
        set.spawn(async move {
            info!("Starting PhashFiles background task");
            let task = phash_files::PhashFiles::new(db, fs);
            task.process(token).await;
            info!("PhashFiles background task completed");
        });
    }

    #[cfg(feature = "labels")]
    {
        if let Some(label_models) = settings_snap.label_models.clone()
            && !label_models.is_empty()
        {
            let db = db.clone();
            let fs = file_store.clone();
            let models_dir = settings_snap
                .models_dir
                .clone()
                .unwrap_or_else(|| fs.storage_dir().join("models"));
            let flag_terms = settings_snap.label_flag_terms.clone().unwrap_or_default();
            let token = shutdown.clone();
            set.spawn(async move {
                info!("Starting LabelFiles background task");
                let task =
                    label_files::LabelFiles::new(db, fs, models_dir, label_models, flag_terms);
                task.process(token).await;
                info!("LabelFiles background task completed");
            });
        }
    }

    #[cfg(feature = "payments")]
    {
        if let Some(client) = client {
            let db = db.clone();
            let token = shutdown.clone();
            set.spawn(async move {
                info!("Starting PaymentsHandler background task");
                let mut m = payments::PaymentsHandler::new(client, db);
                if let Err(e) = m.process(token).await {
                    error!("PaymentsHandler failed: {}", e);
                } else {
                    info!("PaymentsHandler background task completed");
                }
            });
        } else {
            log::warn!("Not starting PaymentsHandler, configuration missing")
        }
    }

    // Always start the file-deleter task; it reads thresholds from live
    // settings each cycle and idles when both policies are disabled.
    {
        let db = db.clone();
        let fs = file_store.clone();
        let live = settings.clone();
        let token = shutdown.clone();
        set.spawn(async move {
            info!("Starting FileDeleter background task");
            let task = file_deleter::FileDeleter::new(db, fs, live);
            task.process(token).await;
            info!("FileDeleter background task completed");
        });
    }

    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idle_returns_idle_sleep_and_resets_state() {
        let mut prev = 5;
        let mut stall = 3;
        let dur = next_sleep(&BatchResult::Idle, &mut prev, &mut stall);
        assert_eq!(dur, IDLE_SLEEP);
        assert_eq!(prev, 0);
        assert_eq!(stall, 0);
    }

    #[test]
    fn test_processed_new_count_returns_batch_sleep() {
        let mut prev = 0;
        let mut stall = 0;
        let dur = next_sleep(&BatchResult::Processed { found: 5 }, &mut prev, &mut stall);
        assert_eq!(dur, BATCH_SLEEP);
        assert_eq!(prev, 5);
        assert_eq!(stall, 0);
    }

    #[test]
    fn test_stall_detection_triggers_backoff() {
        let mut prev = 1;
        let mut stall = 0;

        // First stall: same count as last time -> stall_rounds becomes 1.
        let dur = next_sleep(&BatchResult::Processed { found: 1 }, &mut prev, &mut stall);
        assert_eq!(stall, 1);
        assert!(dur > BATCH_SLEEP, "should back off beyond BATCH_SLEEP");
        assert_eq!(dur, BATCH_SLEEP.saturating_mul(2)); // 2^1 * BATCH_SLEEP
    }

    #[test]
    fn test_backoff_increases_exponentially() {
        let mut prev = 1;
        let mut stall = 0;

        let mut durations = Vec::new();
        for _ in 0..5 {
            let dur = next_sleep(&BatchResult::Processed { found: 1 }, &mut prev, &mut stall);
            durations.push(dur);
        }

        // Each duration should be >= the previous (until the cap).
        for w in durations.windows(2) {
            assert!(w[1] >= w[0], "backoff must be non-decreasing");
        }
    }

    #[test]
    fn test_backoff_capped_at_max() {
        let mut prev = 1;
        let mut stall = 0;

        // Run many rounds to hit the cap.
        let mut dur = Duration::ZERO;
        for _ in 0..50 {
            dur = next_sleep(&BatchResult::Processed { found: 1 }, &mut prev, &mut stall);
        }
        assert_eq!(dur, MAX_BACKOFF);
    }

    #[test]
    fn test_stall_resets_when_count_changes() {
        let mut prev = 1;
        let mut stall = 0;

        // Stall a few rounds.
        for _ in 0..3 {
            next_sleep(&BatchResult::Processed { found: 1 }, &mut prev, &mut stall);
        }
        assert!(stall > 0);

        // Count changes -> stall resets.
        let dur = next_sleep(&BatchResult::Processed { found: 2 }, &mut prev, &mut stall);
        assert_eq!(stall, 0);
        assert_eq!(dur, BATCH_SLEEP);
        assert_eq!(prev, 2);
    }

    #[test]
    fn test_stall_resets_on_idle_after_stall() {
        let mut prev = 1;
        let mut stall = 0;

        // Stall a few rounds.
        for _ in 0..3 {
            next_sleep(&BatchResult::Processed { found: 1 }, &mut prev, &mut stall);
        }
        assert!(stall > 0);

        // Idle -> everything resets.
        let dur = next_sleep(&BatchResult::Idle, &mut prev, &mut stall);
        assert_eq!(dur, IDLE_SLEEP);
        assert_eq!(prev, 0);
        assert_eq!(stall, 0);
    }

    #[test]
    fn test_processed_zero_found_treated_as_progress() {
        // `found == 0` shouldn't trigger stall even if prev was also 0,
        // because `found > 0` is required for the stall check.
        let mut prev = 0;
        let mut stall = 0;
        let dur = next_sleep(&BatchResult::Processed { found: 0 }, &mut prev, &mut stall);
        assert_eq!(dur, BATCH_SLEEP);
        assert_eq!(stall, 0);
    }
}

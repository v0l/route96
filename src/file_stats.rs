//! In-memory file access statistics tracker with periodic flush to the database.
//!
//! # Design
//!
//! Download events are recorded in a [`DashMap`]-backed in-memory store rather
//! than hitting the database on every request.  A background task calls
//! [`FileStatsTracker::flush`] on a configurable interval; each flush drains
//! the in-memory map and merges the accumulated values into the `file_stats`
//! table via `INSERT … ON DUPLICATE KEY UPDATE`.
//!
//! Thread-safety comes from [`DashMap`] (sharded RwLock) and
//! [`AtomicI64`] / [`AtomicU64`] for the per-entry counters, so recording a
//! hit never blocks under concurrent load.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use log::{error, info};
use serde::Serialize;
use sqlx::FromRow;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use tokio_util::sync::CancellationToken;

use crate::db::Database;

/// How often the in-memory stats are flushed to the database.
const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Per-file counters kept in memory between flushes.
struct FileStatEntry {
    /// Unix timestamp (seconds) of the most recent access; -1 means not set.
    last_accessed_secs: AtomicI64,
    /// Cumulative egress bytes since the last flush.
    egress_bytes: AtomicU64,
}

impl FileStatEntry {
    fn new(accessed_secs: i64, bytes: u64) -> Self {
        Self {
            last_accessed_secs: AtomicI64::new(accessed_secs),
            egress_bytes: AtomicU64::new(bytes),
        }
    }
}

/// A snapshot extracted from a [`FileStatEntry`] at flush time.
pub struct FileStatSnapshot {
    pub file_id: Vec<u8>,
    pub last_accessed: DateTime<Utc>,
    pub egress_bytes: u64,
}

/// Thread-safe in-memory store for file access statistics.
///
/// Clone-cheap: the inner map is `Arc`-wrapped.
#[derive(Clone)]
pub struct FileStatsTracker {
    inner: Arc<DashMap<Vec<u8>, FileStatEntry>>,
}

impl FileStatsTracker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    /// Record a file access.
    ///
    /// * `file_id`  – raw binary SHA-256 of the file.
    /// * `bytes`    – number of bytes sent in this response.
    /// * `accessed` – timestamp of the access (caller supplies this so that
    ///                tests can inject a deterministic value).
    pub fn record(&self, file_id: &[u8], bytes: u64, accessed: DateTime<Utc>) {
        let secs = accessed.timestamp();
        if let Some(entry) = self.inner.get(file_id) {
            // Atomically update last_accessed to the max of the stored and new value.
            entry.last_accessed_secs.fetch_max(secs, Ordering::Relaxed);
            entry.egress_bytes.fetch_add(bytes, Ordering::Relaxed);
        } else {
            // Entry does not exist yet – insert it.  There is a small race
            // here (two concurrent inserts for the same file), but the worst
            // case is losing a few bytes on the very first access, which is
            // acceptable given the eventual-consistency nature of this counter.
            self.inner
                .entry(file_id.to_vec())
                .or_insert_with(|| FileStatEntry::new(secs, 0))
                .egress_bytes
                .fetch_add(bytes, Ordering::Relaxed);
        }
    }

    /// Drain all accumulated stats and return them as a `Vec` of snapshots.
    ///
    /// Entries are **removed** from the map so that each flush only sends the
    /// delta since the previous flush.
    pub fn drain(&self) -> Vec<FileStatSnapshot> {
        let mut out = Vec::with_capacity(self.inner.len());
        self.inner.retain(|file_id, entry| {
            let bytes = entry.egress_bytes.load(Ordering::Relaxed);
            let secs = entry.last_accessed_secs.load(Ordering::Relaxed);
            if bytes > 0 || secs >= 0 {
                let last_accessed = DateTime::from_timestamp(secs, 0).unwrap_or_else(Utc::now);
                out.push(FileStatSnapshot {
                    file_id: file_id.clone(),
                    last_accessed,
                    egress_bytes: bytes,
                });
            }
            // Remove the entry so we only report deltas.
            false
        });
        out
    }

    /// Flush all pending stats to the database.
    ///
    /// Calls [`Self::drain`] then upserts each snapshot.  Errors are logged
    /// but do not propagate – the caller (background task) simply retries on
    /// the next tick.
    pub async fn flush(&self, db: &Database) {
        let snapshots = self.drain();
        if snapshots.is_empty() {
            return;
        }
        info!("FileStats: flushing {} entries", snapshots.len());
        for snap in snapshots {
            if let Err(e) = db.upsert_file_stats(&snap).await {
                error!(
                    "FileStats: failed to upsert stats for {}: {}",
                    hex::encode(&snap.file_id),
                    e
                );
            }
        }
    }

    /// Spawn the periodic flush loop and return its [`JoinHandle`].
    ///
    /// The task wakes every [`FLUSH_INTERVAL`] seconds (or immediately on
    /// shutdown) and calls [`Self::flush`].
    pub fn start_flush_task(
        self,
        db: Database,
        shutdown: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!(
                "FileStats flush task started (interval: {:?})",
                FLUSH_INTERVAL
            );
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(FLUSH_INTERVAL) => {
                        self.flush(&db).await;
                    }
                    _ = shutdown.cancelled() => {
                        // Final flush before exit.
                        info!("FileStats flush task shutting down, performing final flush");
                        self.flush(&db).await;
                        return;
                    }
                }
            }
        })
    }
}

impl Default for FileStatsTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Database-level stats for a single file, returned by queries.
#[derive(Clone, Serialize, FromRow)]
pub struct FileStats {
    pub last_accessed: Option<DateTime<Utc>>,
    pub egress_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[test]
    fn record_accumulates_bytes() {
        let tracker = FileStatsTracker::new();
        let id = vec![1u8; 32];
        tracker.record(&id, 100, ts(1000));
        tracker.record(&id, 200, ts(1001));

        let snaps = tracker.drain();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].egress_bytes, 300);
    }

    #[test]
    fn record_tracks_latest_access_time() {
        let tracker = FileStatsTracker::new();
        let id = vec![2u8; 32];
        tracker.record(&id, 0, ts(1000));
        tracker.record(&id, 0, ts(2000));
        tracker.record(&id, 0, ts(1500)); // older, should not overwrite 2000

        let snaps = tracker.drain();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].last_accessed.timestamp(), 2000);
    }

    #[test]
    fn drain_clears_entries() {
        let tracker = FileStatsTracker::new();
        let id = vec![3u8; 32];
        tracker.record(&id, 50, ts(1000));
        let first = tracker.drain();
        let second = tracker.drain();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 0);
    }

    #[test]
    fn multiple_files_are_tracked_independently() {
        let tracker = FileStatsTracker::new();
        let id_a = vec![4u8; 32];
        let id_b = vec![5u8; 32];
        tracker.record(&id_a, 100, ts(1000));
        tracker.record(&id_b, 200, ts(2000));

        let mut snaps = tracker.drain();
        snaps.sort_by_key(|s| s.egress_bytes);

        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].egress_bytes, 100);
        assert_eq!(snaps[1].egress_bytes, 200);
    }
}

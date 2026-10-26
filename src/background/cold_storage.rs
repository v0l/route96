use super::{BatchResult, next_sleep};
use crate::db::{Database, UploadDownloads};
use crate::filesystem::{FileStore, StorageTier};
use crate::settings::Settings;
use chrono::{DateTime, Utc};
use log::{error, info, warn};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

const PAGE_SIZE: u32 = 500;

pub struct ColdStorage {
    db: Database,
    fs: FileStore,
    settings: Arc<RwLock<Settings>>,
    cursor: Option<Vec<u8>>,
}

fn wanted_tier(
    file: &UploadDownloads,
    min_downloads: u64,
    created_before: DateTime<Utc>,
) -> StorageTier {
    let popular = (file.egress_bytes as u128) >= (file.size as u128) * (min_downloads as u128);
    if popular || file.created >= created_before {
        StorageTier::Hot
    } else {
        StorageTier::Cold
    }
}

impl ColdStorage {
    pub fn new(db: Database, fs: FileStore, settings: Arc<RwLock<Settings>>) -> Self {
        Self {
            db,
            fs,
            settings,
            cursor: None,
        }
    }

    pub async fn process(mut self, shutdown: CancellationToken) {
        let mut prev_count: usize = 0;
        let mut stall_rounds: u32 = 0;
        loop {
            let sleep_dur;
            tokio::select! {
                result = self.run_batch(&shutdown) => {
                    sleep_dur = next_sleep(&result, &mut prev_count, &mut stall_rounds);
                    if let BatchResult::Processed { found, failed } = result
                        && stall_rounds > 0
                    {
                        warn!(
                            "ColdStorage: stalled on {} files ({} failed), backing off {:.0?}",
                            found, failed, sleep_dur,
                        );
                    }
                }
                _ = shutdown.cancelled() => return,
            }
            tokio::select! {
                _ = tokio::time::sleep(sleep_dur) => {}
                _ = shutdown.cancelled() => return,
            }
        }
    }

    pub(crate) async fn run_batch(&mut self, shutdown: &CancellationToken) -> BatchResult {
        let (min_downloads, after_days) = {
            let s = self.settings.read().await;
            (s.cold_storage_min_downloads(), s.cold_storage_after_days())
        };
        let created_before = Utc::now() - chrono::Duration::days(after_days as i64);

        let page = match self
            .db
            .get_upload_downloads_page(self.cursor.as_deref(), PAGE_SIZE)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                error!("ColdStorage: failed to list uploads: {}", e);
                return BatchResult::Idle;
            }
        };
        let Some(last) = page.last() else {
            self.cursor = None;
            return BatchResult::Idle;
        };
        self.cursor = Some(last.id.clone());

        let mut found = 0;
        let mut failed = 0;
        for file in &page {
            if shutdown.is_cancelled() {
                break;
            }
            let wanted = wanted_tier(file, min_downloads, created_before);
            match self.fs.tier(&file.id) {
                None => continue,
                Some(current) if current == wanted => continue,
                Some(_) => {}
            }
            found += 1;
            match self.fs.relocate(&file.id, wanted).await {
                Ok(_) => {}
                Err(e) => {
                    error!(
                        "ColdStorage: failed to move {} to {:?}: {}",
                        hex::encode(&file.id),
                        wanted,
                        e
                    );
                    failed += 1;
                }
            }
        }
        if found > 0 {
            info!(
                "ColdStorage: moved {} file(s), {} failed",
                found - failed,
                failed
            );
        }
        BatchResult::Processed { found, failed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::FileUpload;
    use crate::file_stats::FileStatSnapshot;
    use sha2::{Digest, Sha256};
    use sqlx::Executor;

    fn upload(size: u64, egress_bytes: u64, age_days: i64) -> UploadDownloads {
        UploadDownloads {
            id: vec![0; 32],
            size,
            created: Utc::now() - chrono::Duration::days(age_days),
            egress_bytes,
        }
    }

    #[test]
    fn wanted_tier_uses_download_count_and_age() {
        let cutoff = Utc::now() - chrono::Duration::days(7);
        assert_eq!(
            wanted_tier(&upload(10, 490, 30), 50, cutoff),
            StorageTier::Cold
        );
        assert_eq!(
            wanted_tier(&upload(10, 500, 30), 50, cutoff),
            StorageTier::Hot
        );
        assert_eq!(wanted_tier(&upload(10, 0, 1), 50, cutoff), StorageTier::Hot);
        assert_eq!(wanted_tier(&upload(10, 0, 30), 0, cutoff), StorageTier::Hot);
        assert_eq!(
            wanted_tier(&upload(u64::MAX, u64::MAX, 30), 50, cutoff),
            StorageTier::Cold
        );
    }

    async fn test_db() -> Database {
        let url = std::env::var("ROUTE96_TEST_DATABASE")
            .expect("set ROUTE96_TEST_DATABASE=mysql://root:root@127.0.0.1:3399");
        let name = format!("r96_cold_{}", uuid::Uuid::new_v4().simple());
        let admin = Database::new(&url).await.unwrap();
        admin
            .pool
            .execute(format!("create database {}", name).as_str())
            .await
            .unwrap();
        let db = Database::new(&format!("{}/{}", url, name)).await.unwrap();
        db.migrate().await.unwrap();
        db
    }

    fn settings(hot: &std::path::Path, cold: &std::path::Path) -> Arc<RwLock<Settings>> {
        let json = serde_json::json!({
            "storage_dir": hot,
            "cold_storage_dir": cold,
            "database": "",
            "max_upload_bytes": 1024,
            "public_url": "",
            "cold_storage_min_downloads": 5,
            "cold_storage_after_days": 1,
        });
        Arc::new(RwLock::new(serde_json::from_value(json).unwrap()))
    }

    async fn add(
        db: &Database,
        fs: &FileStore,
        content: &[u8],
        age_days: i64,
        downloads: u64,
    ) -> Vec<u8> {
        let id = Sha256::digest(content).to_vec();
        let path = fs.tier_path(&id, StorageTier::Hot).unwrap();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, content).await.unwrap();
        let upload = FileUpload {
            id: id.clone(),
            size: content.len() as u64,
            mime_type: "image/png".to_string(),
            created: Utc::now() - chrono::Duration::days(age_days),
            ..Default::default()
        };
        db.add_file(&upload, None).await.unwrap();
        if downloads > 0 {
            add_downloads(db, &id, content.len() as u64, downloads).await;
        }
        id
    }

    async fn add_downloads(db: &Database, id: &[u8], size: u64, downloads: u64) {
        db.upsert_file_stats(&FileStatSnapshot {
            file_id: id.to_vec(),
            last_accessed: Utc::now(),
            egress_bytes: downloads * size,
        })
        .await
        .unwrap();
    }

    async fn sweep(task: &mut ColdStorage) -> usize {
        let token = CancellationToken::new();
        let mut moved = 0;
        while let BatchResult::Processed { found, failed } = task.run_batch(&token).await {
            assert_eq!(failed, 0);
            moved += found;
        }
        moved
    }

    fn on_disk(fs: &FileStore, id: &Vec<u8>, tier: StorageTier) -> bool {
        fs.tier_path(id, tier).unwrap().exists()
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn moves_files_below_download_threshold_to_cold_and_back() {
        let hot = tempfile::tempdir().unwrap();
        let cold = tempfile::tempdir().unwrap();
        let settings = settings(hot.path(), cold.path());
        let fs = FileStore::new(settings.clone());
        let db = test_db().await;

        let unpopular = add(&db, &fs, b"unpopular", 10, 4).await;
        let popular = add(&db, &fs, b"popular", 10, 5).await;
        let fresh = add(&db, &fs, b"fresh", 0, 0).await;

        let mut task = ColdStorage::new(db.clone(), fs.clone(), settings.clone());
        assert_eq!(sweep(&mut task).await, 1);
        assert_eq!(fs.tier(&unpopular), Some(StorageTier::Cold));
        assert!(on_disk(&fs, &unpopular, StorageTier::Cold));
        assert!(!on_disk(&fs, &unpopular, StorageTier::Hot));
        assert_eq!(
            fs.get(&unpopular),
            fs.tier_path(&unpopular, StorageTier::Cold).unwrap()
        );
        assert!(on_disk(&fs, &popular, StorageTier::Hot));
        assert!(on_disk(&fs, &fresh, StorageTier::Hot));
        assert_eq!(sweep(&mut task).await, 0);

        add_downloads(&db, &unpopular, b"unpopular".len() as u64, 1).await;
        assert_eq!(sweep(&mut task).await, 1);
        assert!(on_disk(&fs, &unpopular, StorageTier::Hot));
        assert!(!on_disk(&fs, &unpopular, StorageTier::Cold));

        settings.write().await.cold_storage_min_downloads = Some(10);
        assert_eq!(sweep(&mut task).await, 2);
        assert!(on_disk(&fs, &unpopular, StorageTier::Cold));
        assert!(on_disk(&fs, &popular, StorageTier::Cold));

        let restarted = FileStore::new(settings.clone());
        assert_eq!(restarted.tier(&popular), Some(StorageTier::Cold));

        settings.write().await.cold_storage_min_downloads = Some(0);
        assert_eq!(sweep(&mut task).await, 2);
        assert!(on_disk(&fs, &unpopular, StorageTier::Hot));
        assert!(on_disk(&fs, &popular, StorageTier::Hot));
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn file_missing_from_disk_is_skipped() {
        let hot = tempfile::tempdir().unwrap();
        let cold = tempfile::tempdir().unwrap();
        let settings = settings(hot.path(), cold.path());
        let fs = FileStore::new(settings.clone());
        let db = test_db().await;
        let id = add(&db, &fs, b"gone", 10, 0).await;
        fs.delete(&id).await.unwrap();

        let mut task = ColdStorage::new(db.clone(), fs.clone(), settings);
        assert_eq!(sweep(&mut task).await, 0);
        assert_eq!(fs.tier(&id), None);
    }
}

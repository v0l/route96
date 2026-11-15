mod few_upload_non_media;
mod hard_age;
mod inactivity;
mod zero_egress;

use crate::db::Database;
use crate::settings::Settings;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub use few_upload_non_media::FewUploadNonMedia;
pub use hard_age::HardAge;
pub use inactivity::Inactivity;
pub use zero_egress::ZeroEgress;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: Vec<u8>,
    pub reason: &'static str,
}

#[async_trait]
pub trait CleanupHeuristic: Send + Sync {
    fn name(&self) -> &'static str;

    fn enabled(&self, settings: &Settings) -> bool;

    async fn candidates(
        &self,
        db: &Database,
        settings: &Settings,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<Candidate>>;
}

pub fn default_heuristics() -> Vec<Box<dyn CleanupHeuristic>> {
    vec![
        Box::new(Inactivity),
        Box::new(HardAge),
        Box::new(ZeroEgress),
        Box::new(FewUploadNonMedia),
    ]
}

pub(crate) fn days_cutoff(now: DateTime<Utc>, days: u64) -> DateTime<Utc> {
    now - chrono::Duration::seconds((days * 86_400) as i64)
}

pub(crate) fn active_days(days: Option<u64>) -> Option<u64> {
    days.filter(|&d| d > 0)
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::db::{Database, FileUpload};
    use crate::settings::Settings;
    use chrono::{DateTime, Utc};
    use sha2::{Digest, Sha256};
    use sqlx::Executor;

    pub async fn test_db() -> Database {
        let url = std::env::var("ROUTE96_TEST_DATABASE")
            .expect("set ROUTE96_TEST_DATABASE=mysql://root:root@127.0.0.1:3399");
        let name = format!("r96_cleanup_{}", uuid::Uuid::new_v4().simple());
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

    pub fn settings() -> Settings {
        serde_json::from_value(serde_json::json!({
            "storage_dir": "/tmp",
            "database": "",
            "max_upload_bytes": 1024,
            "public_url": "",
        }))
        .unwrap()
    }

    pub async fn add_file(
        db: &Database,
        content: &str,
        mime: &str,
        created: DateTime<Utc>,
        owner: Option<u64>,
    ) -> Vec<u8> {
        let id = Sha256::digest(content.as_bytes()).to_vec();
        let upload = FileUpload {
            id: id.clone(),
            size: content.len() as u64,
            mime_type: mime.to_string(),
            created,
            ..Default::default()
        };
        db.add_file(&upload, owner).await.unwrap();
        id
    }

    pub async fn add_user(db: &Database, seed: u8) -> u64 {
        db.upsert_user(&vec![seed; 32]).await.unwrap()
    }

    pub fn sorted(mut v: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        v.sort();
        v
    }
}

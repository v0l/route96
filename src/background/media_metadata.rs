use crate::db::{Database, FileUpload};
use crate::filesystem::FileStore;
use crate::processing::probe_file;
use anyhow::Result;
use log::{error, info, warn};
use tokio::sync::broadcast::Receiver;

pub struct MediaMetadata {
    db: Database,
    fs: FileStore,
}

impl MediaMetadata {
    pub fn new(db: Database, fs: FileStore) -> Self {
        Self { db, fs }
    }

    pub async fn process(&mut self, mut shutdown: Receiver<()>) -> Result<()> {
        let to_migrate = self.db.get_missing_media_metadata().await?;

        info!("{} files are missing metadata", to_migrate.len());

        for file in to_migrate {
            if shutdown.try_recv().is_ok() {
                break;
            }
            // probe file and update metadata
            let path = self.fs.get(&file.id);
            match probe_file(&path) {
                Ok(data) => {
                    let bv = data.best_video();
                    let duration = if data.duration < 0.0 {
                        None
                    } else {
                        Some(data.duration)
                    };
                    let bitrate = if data.bitrate == 0 {
                        None
                    } else {
                        Some(data.bitrate as u32)
                    };
                    info!(
                        "Updating metadata: id={}, dim={}x{}, dur={}, br={}",
                        hex::encode(&file.id),
                        bv.map(|v| v.width).unwrap_or(0),
                        bv.map(|v| v.height).unwrap_or(0),
                        duration.unwrap_or(0.0),
                        bitrate.unwrap_or(0)
                    );
                    if let Err(e) = self
                        .db
                        .update_metadata(
                            &file.id,
                            bv.map(|v| v.width as u32),
                            bv.map(|v| v.height as u32),
                            duration,
                            bitrate,
                        )
                        .await
                    {
                        error!("Failed to update metadata: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Skipping missing file: {}, {}", hex::encode(&file.id), e);
                }
            }
        }
        Ok(())
    }
}

impl Database {
    pub async fn get_missing_media_metadata(&mut self) -> Result<Vec<FileUpload>> {
        let results: Vec<FileUpload> = sqlx::query_as("select * from uploads where \
                          (mime_type like 'image/%' and (width is null or height is null)) or \
                           (mime_type like 'video/%' and (width is null or height is null or bitrate is null or duration is null))")
                .fetch_all(&self.pool)
                .await?;

        Ok(results)
    }

    pub async fn update_metadata(
        &mut self,
        id: &Vec<u8>,
        width: Option<u32>,
        height: Option<u32>,
        duration: Option<f32>,
        bitrate: Option<u32>,
    ) -> Result<()> {
        sqlx::query("update uploads set width=?, height=?, duration=?, bitrate=? where id=?")
            .bind(width)
            .bind(height)
            .bind(duration)
            .bind(bitrate)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

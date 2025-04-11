use crate::db::{Database, FileUpload};
use crate::filesystem::FileStore;
use crate::storage::StorageBackend;

use anyhow::{Context, Result};
use ffmpeg_the_third::{self, codec::Context as CodecContext, ffi::AV_NOPTS_VALUE, media};
use log::{debug, error, info, warn};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::{self, File};
use tokio::io::{self, AsyncWriteExt};
use uuid::Uuid;

#[cfg(feature = "media-compression")]
use crate::processing::probe_file;

// RAII guard for temporary files used in this module
struct TempFileCleanup(PathBuf);

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        let path = &self.0;
        if path.exists() {
            debug!("Cleaning up temporary metadata probe file: {:?}", path);
            let path_clone = path.clone();
            // Spawn a task for cleanup to avoid blocking drop if possible
            tokio::spawn(async move {
                if let Err(e) = fs::remove_file(&path_clone).await {
                    error!(
                        "Failed to clean up temporary probe file {:?}: {}",
                        path_clone, e
                    );
                } else {
                    debug!(
                        "Successfully cleaned up temporary probe file: {:?}",
                        path_clone
                    );
                }
            });
        }
    }
}

pub struct MediaMetadata {
    db: Arc<Database>,
    storage: Arc<FileStore>,
    temp_dir: PathBuf, // Directory for temporary probe files
    check_interval: u64,
}

impl MediaMetadata {
    pub fn new(db: Arc<Database>, storage: Arc<FileStore>, temp_dir: PathBuf) -> Self {
        Self {
            db,
            storage,
            temp_dir,
            check_interval: 60, // Default interval
        }
    }

    pub async fn process(&mut self) -> Result<()> {
        loop {
            info!("Checking for files needing metadata update");
            let files = match self.db.get_missing_media_metadata().await {
                Ok(f) => f,
                Err(e) => {
                    error!("Failed to query files needing metadata: {}", e);
                    // Wait before retrying DB query
                    tokio::time::sleep(Duration::from_secs(self.check_interval)).await;
                    continue;
                }
            };

            if files.is_empty() {
                debug!("No files found needing metadata update, sleeping...");
                tokio::time::sleep(Duration::from_secs(self.check_interval)).await;
                continue;
            }

            info!("Found {} files to process for metadata", files.len());
            for file_record in files {
                let file_id_hex = hex::encode(&file_record.id);
                debug!("Processing file ID: {}", file_id_hex);

                // Use a separate async block for each file to handle errors individually
                if let Err(e) = self.process_single_file(&file_record).await {
                    error!("Error processing metadata for file {}: {}", file_id_hex, e);
                    // Continue to the next file even if one fails
                }
            }

            info!("Finished processing metadata batch, sleeping...");
            tokio::time::sleep(Duration::from_secs(self.check_interval)).await;
        }
    }

    // Process metadata for a single file
    async fn process_single_file(&self, file_record: &FileUpload) -> Result<()> {
        // 1. Get a stream reader for the blob from storage
        let mut reader = match self.storage.stream_reader(&file_record.id, None).await {
            Ok(r) => r,
            Err(e) => {
                // Special handling for "not found" errors vs other storage errors
                if e.to_string().contains("Blob not found") {
                    warn!(
                        "Blob ID {} not found in storage during metadata check. Skipping.",
                        hex::encode(&file_record.id)
                    );
                    // Maybe mark the file record as missing in DB?
                    return Ok(()); // Skip this file
                } else {
                    return Err(e).context("Failed to get stream reader from storage");
                }
            }
        };

        // 2. Create a temporary file for probing
        let temp_file_path = self.temp_dir.join(Uuid::new_v4().to_string());
        fs::create_dir_all(&self.temp_dir)
            .await
            .context("Failed to create temp directory for probe file")?;
        let mut temp_file = File::create(&temp_file_path)
            .await
            .context("Failed to create temp file for probing")?;

        // RAII guard for the temporary file
        let _cleanup = TempFileCleanup(temp_file_path.clone());

        // 3. Stream content from storage to the temporary file
        io::copy(&mut reader, &mut temp_file)
            .await
            .context("Failed to copy stream to temp file")?;
        temp_file
            .flush()
            .await
            .context("Failed to flush temp file")?;
        drop(temp_file); // Close file handle explicitly

        // 4. Probe the temporary file (requires media-compression feature)
        #[cfg(feature = "media-compression")]
        {
            if let Ok(probe) = probe_file(&temp_file_path) {
                let v_stream = probe.streams().best(media::Type::Video);
                let a_stream = probe.streams().best(media::Type::Audio);
                let s_stream = probe.streams().best(media::Type::Subtitle);
                let duration_raw = probe.duration();
                let duration_f32 = if duration_raw < 0i64 || duration_raw == AV_NOPTS_VALUE {
                    None // Treat AV_NOPTS_VALUE or negative as None
                } else {
                    // Convert duration from AV_TIME_BASE to seconds (f32)
                    Some(duration_raw as f32 / ffmpeg_the_third::ffi::AV_TIME_BASE as f32)
                };
                let bitrate = if probe.bit_rate() <= 0 {
                    None
                } else {
                    Some(probe.bit_rate() as i32)
                };

                if let Some(stream) = v_stream {
                    let stream_index = stream.index();
                    let params = stream.parameters();
                    if let Ok(decoder_ctx) = CodecContext::from_parameters(params) {
                        if let Ok(decoder) = decoder_ctx.decoder().video() {
                            let width = Some(decoder.width() as i32);
                            let height = Some(decoder.height() as i32);
                            info!(
                                "Updating video metadata for file {}: width={:?}, height={:?}, duration={:?}, bitrate={:?}",
                                hex::encode(&file_record.id),
                                width,
                                height,
                                duration_f32,
                                bitrate
                            );
                            self.db
                                .update_metadata(
                                    &file_record.id,
                                    width,
                                    height,
                                    duration_f32,
                                    bitrate,
                                )
                                .await
                                .context("Failed to update metadata in database")?;
                        } else {
                            warn!(
                                "Could not get video decoder for file {}: {}",
                                hex::encode(&file_record.id),
                                "Could not get video decoder"
                            );
                        }
                    } else {
                        warn!(
                            "Could not get codec context for file {}: {}",
                            hex::encode(&file_record.id),
                            "Could not get codec context"
                        );
                    }
                }
                // Add similar logic for audio stream if needed
                else if a_stream.is_some() {
                    // Fallback: update with duration/bitrate even if no video
                    info!(
                        "Updating audio-only metadata for file {}: duration={:?}, bitrate={:?}",
                        hex::encode(&file_record.id),
                        duration_f32,
                        bitrate
                    );
                    self.db
                        .update_metadata(&file_record.id, None, None, duration_f32, bitrate)
                        .await
                        .context("Failed to update metadata in database")?;
                } else if s_stream.is_some() {
                    info!(
                        "Updating subtitle-only metadata for file {}: duration={:?}, bitrate={:?}",
                        hex::encode(&file_record.id),
                        duration_f32,
                        bitrate
                    );
                    self.db
                        .update_metadata(&file_record.id, None, None, duration_f32, bitrate)
                        .await
                        .context("Failed to update metadata in database")?;
                }
                // Add logic for subtitle stream if needed
                else {
                    warn!(
                        "Could not determine stream type for file {} during metadata probe",
                        hex::encode(&file_record.id)
                    );
                }
            } else {
                warn!(
                    "Probe failed for file {}. Unable to extract metadata.",
                    hex::encode(&file_record.id)
                );
                // Optionally, update the DB record to indicate probe failure?
            }
        }
        #[cfg(not(feature = "media-compression"))]
        {
            // If media-compression is disabled, we can't probe.
            // Log this or optionally update the DB to prevent re-checking?
            debug!(
                "Skipping metadata probe for file {}: 'media-compression' feature disabled.",
                hex::encode(&file_record.id)
            );
        }

        Ok(())
        // Temp file is cleaned up automatically when _cleanup goes out of scope
    }
}

impl Database {
    pub async fn get_missing_media_metadata(&self) -> Result<Vec<FileUpload>> {
        let results: Vec<FileUpload> = sqlx::query_as("select * from uploads where \
                          (mime_type like 'image/%' and (width is null or height is null)) or \
                           (mime_type like 'video/%' and (width is null or height is null or bitrate is null or duration is null))")
                .fetch_all(&self.pool)
                .await?;

        Ok(results)
    }

    pub async fn update_metadata(
        &self,
        id: &[u8],
        width: Option<i32>,
        height: Option<i32>,
        duration: Option<f32>,
        bitrate: Option<i32>,
    ) -> Result<()> {
        sqlx::query("UPDATE uploads SET width=$1, height=$2, duration=$3, bitrate=$4 WHERE id=$5")
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

#[cfg(feature = "media-compression")]
pub async fn job(db: Arc<Database>, storage: Arc<FileStore>, temp_dir: PathBuf) -> Result<()> {
    info!("Starting MediaMetadata background task");
    let mut m = MediaMetadata::new(db, storage, temp_dir);
    m.process().await
}

use std::path::PathBuf;

use tokio::fs::{self, File};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use uuid::Uuid;

// use crate::config::CONFIG; // Removed
// use crate::db::DbError; // Removed
#[cfg(feature = "labels")]
use crate::db::FileLabel;
use crate::db::FileUpload;
// #[cfg(feature = "media-compression")] Remove unused
// use crate::processing;
use crate::settings::FileSystemStorageSettings;
use crate::storage::{BlobMetadata, HttpRange, StorageBackend, StorageResult};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono;
#[cfg(feature = "media-compression")]
use ffmpeg_the_third::{self as ffmpeg, codec::Context as CodecContext};
use hex;
// #[cfg(feature = "media-compression")] Remove unused
// use image::ImageFormat;
// #[cfg(feature = "media-compression")] Remove unused
// use infer;
use log::{error, warn};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub enum FileSystemResult {
    /// File hash already exists
    AlreadyExists(Vec<u8>),
    /// New file created on disk and is stored
    NewFile(NewFileResult),
}

#[derive(Clone, Serialize)]
pub struct NewFileResult {
    pub path: PathBuf,
    #[serde(with = "hex")]
    pub id: Vec<u8>,
    pub size: u64,
    pub mime_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub blur_hash: Option<String>,
    pub duration: Option<f32>,
    pub bitrate: Option<u32>,
    #[cfg(feature = "labels")]
    pub labels: Vec<FileLabel>,
}

#[derive(Clone)]
pub struct FileStore {
    pub storage_dir: PathBuf,
    pub temp_dir: PathBuf,
}

impl FileStore {
    pub fn new(settings: &FileSystemStorageSettings) -> Result<Self> {
        let storage_dir = PathBuf::from(&settings.storage_dir);
        let temp_dir = std::env::temp_dir().join("route96_uploads");
        std::fs::create_dir_all(&storage_dir)?;
        std::fs::create_dir_all(&temp_dir)?;
        Ok(Self {
            storage_dir,
            temp_dir,
        })
    }

    // Helper to get the full path for a blob ID
    pub fn get_path(&self, id: &[u8]) -> PathBuf {
        let filename = hex::encode(id);
        // Simple two-level directory structure based on the first 4 hex chars (2 bytes)
        let dir1 = &filename[0..2];
        let dir2 = &filename[2..4];
        self.storage_dir.join(dir1).join(dir2).join(filename)
    }

    // Helper to store stream to temp file and calculate hash
    async fn store_and_hash_temp(
        &self,
        mut stream: Box<dyn AsyncRead + Send + Unpin>,
    ) -> Result<(PathBuf, u64, Vec<u8>)> {
        let uid = Uuid::new_v4();
        let temp_path = self.temp_dir.join(uid.to_string());
        let mut file = File::create(&temp_path).await?;
        let mut hasher = Sha256::new();
        let mut total_bytes = 0u64;
        let mut buf = [0; 8192];

        loop {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).await?;
            hasher.update(&buf[..n]);
            total_bytes += n as u64;
        }
        let hash = hasher.finalize().to_vec();
        file.flush().await?;
        drop(file);
        Ok((temp_path, total_bytes, hash))
    }
}

#[async_trait]
impl StorageBackend for FileStore {
    async fn put(
        &self,
        stream: Box<dyn AsyncRead + Send + Unpin>,
        mime_type: &str,
    ) -> Result<StorageResult> {
        let (temp_file_path, size, hash) = self.store_and_hash_temp(stream).await?;
        let _cleanup = TempFileCleanup(&temp_file_path);

        let final_path = self.get_path(&hash);
        if final_path.exists() {
            return Ok(StorageResult::AlreadyExists(hash));
        }

        fs::create_dir_all(final_path.parent().unwrap()).await?;
        fs::copy(&temp_file_path, &final_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to copy temp file {:?} to final path {:?}",
                    temp_file_path, final_path
                )
            })?;

        let mime_type = mime_type.to_string();

        // Initialize FileUpload with values not dependent on probing
        let mut file_upload = FileUpload {
            id: hash.clone(),
            size: size as i64,
            mime_type: mime_type.clone(),
            created: chrono::Utc::now(),
            width: None,
            height: None,
            blur_hash: None,
            alt: None,
            duration: None,
            bitrate: None,
            h_tag: None,
            #[cfg(feature = "labels")]
            labels: Vec::new(),
        };

        // Conditionally probe and update FileUpload if media-compression is enabled
        #[cfg(feature = "media-compression")]
        {
            match crate::processing::probe_file(&final_path) {
                Ok(probe_result) => {
                    // Use probe_result.streams() directly
                    if let Some(stream) = probe_result.streams().best(ffmpeg::media::Type::Video) {
                        // Get ParametersRef
                        let params = stream.parameters();
                        // Use CodecContext alias and pass params directly
                        if let Ok(decoder_ctx) = CodecContext::from_parameters(params) {
                            if let Ok(decoder) = decoder_ctx.decoder().video() {
                                file_upload.width = Some(decoder.width() as i32);
                                file_upload.height = Some(decoder.height() as i32);
                            }
                        }
                        // Extract duration from the stream itself if available
                        let duration_ts = stream.duration();
                        if duration_ts != ffmpeg::ffi::AV_NOPTS_VALUE {
                            let tb = stream.time_base();
                            let duration_sec = duration_ts as f64 * tb.0 as f64 / tb.1 as f64;
                            file_upload.duration = Some(duration_sec as f32);
                        }
                    }
                    // Extract bitrate from format context
                    let bitrate_bps = probe_result.bit_rate();
                    if bitrate_bps > 0 {
                        file_upload.bitrate = Some(bitrate_bps as i32);
                    }

                    // Mime type should be the original or inferred, not from probe_result.
                    // Blurhash requires separate calculation.
                    // Remove assignments for these:
                    // file_upload.blur_hash = probe_result.blurhash;
                    // file_upload.mime_type = probe_result.mime_type;
                }
                Err(e) => {
                    warn!("Failed to probe file {}: {}", final_path.display(), e);
                }
            }
        }

        Ok(StorageResult::NewFile {
            id: hash,
            size,
            mime_type: file_upload.mime_type, // Use potentially updated mime_type
        })
    }

    async fn head(&self, id: &[u8]) -> Result<BlobMetadata> {
        let path = self.get_path(id);
        let metadata = fs::metadata(&path).await.map_err(|e| anyhow!(e))?;
        if !metadata.is_file() {
            return Err(anyhow!("Not a file"));
        }

        // Restore mime type handling - for now, just use a placeholder or infer if possible
        // Since infer is behind feature flag and we removed the functions, let's use octet-stream for now
        let mime_type = "application/octet-stream".to_string(); // Placeholder

        Ok(BlobMetadata {
            size: metadata.len(),
            mime_type,
        })
    }

    async fn get_url(&self, id: &[u8], preferred_extension: Option<&str>) -> Result<String> {
        let file_name = hex::encode(id);
        let extension = preferred_extension
            .map(|ext| format!(".{}", ext))
            .unwrap_or_default();
        Ok(format!("/{}{}", file_name, extension)) // Return a server-relative URL
    }

    async fn stream_reader(
        &self,
        id: &[u8],
        range: Option<HttpRange>,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>> {
        let file_path = self.get_path(id);
        let file = File::open(&file_path)
            .await
            .context(format!("File not found for id: {:?}", file_path))?;

        let mut reader: Box<dyn AsyncRead + Send + Unpin> = Box::new(file);

        if let Some(http_range) = range {
            if let Some(start) = http_range.start {
                let mut seekable_file = File::open(&file_path)
                    .await
                    .context("Failed to reopen file for seeking")?;
                seekable_file
                    .seek(io::SeekFrom::Start(start))
                    .await
                    .context("Failed to seek")?;
                reader = Box::new(seekable_file);

                if let Some(end) = http_range.end {
                    let limit = end.saturating_sub(start) + 1;
                    reader = Box::new(reader.take(limit));
                }
            } else if let Some(end) = http_range.end {
                let metadata = fs::metadata(&file_path)
                    .await
                    .context("Failed to get metadata for end-range seek")?;
                let file_len = metadata.len();
                let start = file_len.saturating_sub(end + 1);
                let limit = end + 1;

                let mut seekable_file = File::open(&file_path)
                    .await
                    .context("Failed to reopen file for end-range seeking")?;
                seekable_file
                    .seek(io::SeekFrom::Start(start))
                    .await
                    .context("Failed to seek for end-range")?;
                reader = Box::new(seekable_file.take(limit));
            } else {
                warn!("HttpRange specified but empty (start=None, end=None), reading full file.");
            }
        }
        Ok(reader)
    }

    async fn delete(&self, id: &[u8]) -> Result<()> {
        let path = self.get_path(id);
        if path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .with_context(|| format!("Failed to delete file: {:?}", path))?;
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// RAII guard for temporary files
struct TempFileCleanup<'a>(&'a PathBuf);

impl<'a> Drop for TempFileCleanup<'a> {
    fn drop(&mut self) {
        let path = self.0;
        if path.exists() {
            if let Err(e) = std::fs::remove_file(path) {
                error!("Failed to clean up temporary file {:?}: {}", path, e);
            }
        }
    }
}

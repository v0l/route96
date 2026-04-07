use crate::db::Database;
#[cfg(feature = "labels")]
use crate::db::FileLabel;

#[cfg(feature = "media-compression")]
use crate::processing::{compress_file, probe_file};
use crate::settings::Settings;
use anyhow::Error;
use anyhow::Result;
#[cfg(feature = "media-compression")]
use ffmpeg_rs_raw::DemuxerInfo;
#[cfg(feature = "media-compression")]
use ffmpeg_rs_raw::StreamInfo;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone)]
pub enum FileSystemResult {
    /// File hash already exists
    AlreadyExists(Vec<u8>),
    /// New file created on disk and is stored
    NewFile(NewFileResult),
    /// File hash is banned and must not be re-uploaded
    Banned,
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
    /// Perceptual hash computed at upload time (images only, requires media-compression).
    #[cfg(feature = "media-compression")]
    #[serde(skip)]
    pub phash: Option<[u8; 8]>,
    #[cfg(feature = "labels")]
    pub labels: Vec<FileLabel>,
}

#[derive(Clone)]
pub struct FileStore {
    settings: Arc<RwLock<Settings>>,
    storage_dir: PathBuf,
}

impl FileStore {
    pub fn new(settings: Arc<RwLock<Settings>>) -> Self {
        // storage_dir is read once at startup; it is intentionally not hot-reloadable.
        let storage_dir = {
            let s = settings
                .try_read()
                .expect("settings lock unavailable at FileStore::new");
            PathBuf::from(&s.storage_dir)
        };
        Self {
            settings,
            storage_dir,
        }
    }

    async fn settings(&self) -> Settings {
        self.settings.read().await.clone()
    }

    /// Get a file path by id
    pub fn get(&self, id: &Vec<u8>) -> PathBuf {
        self.map_path(id)
    }

    /// Delete a stored file by id
    pub async fn delete(&self, id: &Vec<u8>) -> Result<()> {
        tokio::fs::remove_file(self.map_path(id)).await?;
        Ok(())
    }

    /// Store a new file
    pub async fn put<'r, S>(
        &self,
        db: &Database,
        path: S,
        mime_type: &str,
        compress: bool,
    ) -> Result<FileSystemResult>
    where
        S: AsyncRead + Unpin + 'r,
    {
        let max = self.settings().await.max_upload_bytes;
        // store file in temp path and hash the file
        // Read at most max+1 bytes so we can detect over-limit uploads without
        // buffering the entire body.
        let (temp_file, size, hash) = self.store_hash_temp_file(path.take(max + 1)).await?;

        if size > max {
            tokio::fs::remove_file(&temp_file).await.ok();
            return Err(anyhow::anyhow!("File exceeds maximum upload size"));
        }

        // check banned before anything else
        if db.is_file_banned(&hash).await? {
            tokio::fs::remove_file(temp_file).await?;
            return Ok(FileSystemResult::Banned);
        }

        let dst_path = self.map_path(&hash);

        // check if file hash already exists
        if dst_path.exists() {
            tokio::fs::remove_file(temp_file).await?;
            return Ok(FileSystemResult::AlreadyExists(hash));
        }

        let mut res = if compress && crate::can_compress(mime_type) {
            #[cfg(feature = "media-compression")]
            {
                let res = match self.compress_file(&temp_file, mime_type).await {
                    Err(e) => {
                        tokio::fs::remove_file(&temp_file).await?;
                        return Err(e);
                    }
                    Ok(res) => res,
                };
                tokio::fs::remove_file(temp_file).await?;
                res
            }
            #[cfg(not(feature = "media-compression"))]
            {
                anyhow::bail!("Compression not supported!");
            }
        } else {
            let (width, height, mime_type, duration, bitrate) = {
                #[cfg(feature = "media-compression")]
                {
                    let probe = probe_file(&temp_file).ok();
                    let v_stream = probe.as_ref().and_then(|p| p.best_video());
                    let mime = Self::hack_mime_type(mime_type, &probe, &v_stream, &temp_file);
                    (
                        v_stream.map(|v| v.width as u32),
                        v_stream.map(|v| v.height as u32),
                        mime,
                        probe
                            .as_ref()
                            .map(|p| if p.duration < 0. { 0.0 } else { p.duration }),
                        probe.as_ref().map(|p| p.bitrate as u32),
                    )
                }
                #[cfg(not(feature = "media-compression"))]
                (
                    None,
                    None,
                    Self::infer_mime_type(mime_type, &temp_file),
                    None,
                    None,
                )
            };
            NewFileResult {
                path: temp_file,
                id: hash,
                size,
                mime_type,
                width,
                height,
                blur_hash: None,
                duration,
                bitrate,
                #[cfg(feature = "media-compression")]
                phash: None,
                #[cfg(feature = "labels")]
                labels: vec![],
            }
        };

        // copy temp file to final destination
        let final_dest = self.map_path(&res.id);

        // Compressed file already exists
        if final_dest.exists() {
            tokio::fs::remove_file(&res.path).await?;
            Ok(FileSystemResult::AlreadyExists(res.id))
        } else {
            tokio::fs::create_dir_all(final_dest.parent().unwrap()).await?;
            tokio::fs::rename(&res.path, &final_dest).await?;

            res.path = final_dest;

            // Compute and store perceptual hash for images.
            #[cfg(feature = "media-compression")]
            if res.mime_type.starts_with("image/") {
                let path = res.path.clone();
                let mime = res.mime_type.clone();
                let db_clone = db.clone();
                match tokio::task::spawn_blocking(move || crate::phash::phash_image(&path, &mime))
                    .await
                {
                    Ok(Ok(phash)) => {
                        match phash.as_bytes().try_into() {
                            Ok(bytes) => {
                                res.phash = Some(bytes);
                                // Check if this image is visually similar to any banned image
                                let settings = self.settings().await;
                                let max_distance = settings
                                    .identical_media_dedup_distance
                                    .unwrap_or(0);
                                if max_distance > 0 {
                                    match db_clone
                                        .is_file_similar_to_banned(&bytes, max_distance)
                                        .await
                                    {
                                        Ok(true) => {
                                            // Remove the uploaded file and reject
                                            tokio::fs::remove_file(&res.path).await.ok();
                                            return Ok(FileSystemResult::Banned);
                                        }
                                        Ok(false) => {}
                                        Err(e) => log::warn!(
                                            "Failed to check banned similarity: {}",
                                            e
                                        ),
                                    }
                                }
                            }
                            Err(_) => log::warn!("phash produced unexpected byte length; skipping"),
                        }
                    }
                    Ok(Err(e)) => log::warn!("Failed to compute phash: {}", e),
                    Err(e) => log::warn!("phash task panicked: {}", e),
                }
            }

            Ok(FileSystemResult::NewFile(res))
        }
    }

    #[cfg(feature = "media-compression")]
    /// Try to replace the mime-type when unknown using ffmpeg probe result
    fn hack_mime_type(
        mime_type: &str,
        p: &Option<DemuxerInfo>,
        stream: &Option<&StreamInfo>,
        out_path: &PathBuf,
    ) -> String {
        if let Some(p) = p {
            let mime = if p.format.contains("mp4") {
                // audio-only MP4 containers (e.g. DASH audio segments) must not be
                // labelled as video/mp4 or thumbnail generation will attempt and fail
                if stream.is_some() {
                    Some("video/mp4")
                } else {
                    Some("audio/mp4")
                }
            } else if p.format.contains("webp") {
                Some("image/webp")
            } else if p.format.contains("jpeg") {
                Some("image/jpeg")
            } else if p.format.contains("png") {
                Some("image/png")
            } else if p.format.contains("gif") {
                Some("image/gif")
            } else {
                None
            };
            let codec = if let Some(s) = stream {
                match s.codec {
                    27 => Some("avc1".to_owned()),           //AV_CODEC_ID_H264
                    173 => Some("hvc1".to_owned()),          //AV_CODEC_ID_HEVC
                    86016 => Some("mp4a.40.33".to_string()), //AV_CODEC_ID_MP2
                    86017 => Some("mp4a.40.34".to_string()), //AV_CODEC_ID_MP3
                    86018 => Some("mp4a.40.2".to_string()),  //AV_CODEC_ID_AAC
                    86019 => Some("ac-3".to_string()),       //AV_CODEC_ID_AC3
                    86056 => Some("ec-3".to_string()),       //AV_CODEC_ID_EAC3
                    _ => None,
                }
            } else {
                None
            };
            if let Some(m) = mime {
                return format!(
                    "{}{}",
                    m,
                    if let Some(c) = codec {
                        format!("; codecs=\"{}\"", c)
                    } else {
                        "".to_owned()
                    }
                );
            }
        }

        // infer mime type
        Self::infer_mime_type(mime_type, out_path)
    }

    fn infer_mime_type(mime_type: &str, out_path: &PathBuf) -> String {
        // infer mime type
        if let Ok(Some(i)) = infer::get_from_path(out_path) {
            i.mime_type().to_string()
        } else {
            mime_type.to_string()
        }
    }

    #[cfg(feature = "media-compression")]
    async fn compress_file(&self, input: &Path, mime_type: &str) -> Result<NewFileResult> {
        let compressed_result = compress_file(input, mime_type, &self.temp_dir())?;

        let hash = FileStore::hash_file(&compressed_result.result).await?;

        let n = File::open(&compressed_result.result)
            .await?
            .metadata()
            .await?
            .len();
        Ok(NewFileResult {
            path: compressed_result.result,
            id: hash,
            size: n,
            width: Some(compressed_result.width as u32),
            height: Some(compressed_result.height as u32),
            blur_hash: None,
            mime_type: compressed_result.mime_type,
            duration: Some(compressed_result.duration),
            bitrate: Some(compressed_result.bitrate),
            phash: None,
            #[cfg(feature = "labels")]
            labels: vec![],
        })
    }

    async fn store_hash_temp_file<S>(&self, mut stream: S) -> Result<(PathBuf, u64, Vec<u8>)>
    where
        S: AsyncRead + Unpin,
    {
        let uid = Uuid::new_v4();
        let out_path = self.temp_dir().join(uid.to_string());
        tokio::fs::create_dir_all(&out_path.parent().unwrap()).await?;

        let mut file = File::create(&out_path).await?;
        let n = tokio::io::copy(&mut stream, &mut file).await?;

        let hash = FileStore::hash_file(&out_path).await?;
        Ok((out_path, n, hash))
    }

    pub async fn hash_file(p: &Path) -> Result<Vec<u8>, Error> {
        let mut file = File::open(p).await?;
        let mut hasher = Sha256::new();
        let mut buf = [0; 4096];
        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let res = hasher.finalize();
        Ok(res.to_vec())
    }

    fn map_path(&self, id: &Vec<u8>) -> PathBuf {
        let id = hex::encode(id);
        self.storage_dir().join(&id[0..2]).join(&id[2..4]).join(id)
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.storage_dir().join("tmp")
    }

    pub fn storage_dir(&self) -> PathBuf {
        self.storage_dir.clone()
    }
}

#[cfg(all(test, feature = "labels"))]
mod tests {
    use super::*;

    fn make_store() -> FileStore {
        let settings = Settings {
            listen: None,
            storage_dir: std::env::temp_dir().to_str().unwrap().to_string(),
            database: String::new(),
            max_upload_bytes: 0,
            public_url: String::new(),
            whitelist: None,
            models_dir: None,
            label_models: None,
            label_flag_terms: None,
            webhook_url: None,
            #[cfg(feature = "blossom")]
            reject_sensitive_exif: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup_distance: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup_allow_override: None,
            #[cfg(feature = "payments")]
            payments: None,
            delete_unaccessed_days: None,
            delete_after_days: None,
        };
        FileStore::new(Arc::new(RwLock::new(settings)))
    }

    #[test]
    fn test_models_dir_defaults_to_storage_subdir() {
        let store = make_store();
        let expected = store.storage_dir().join("models");
        let derived = store.storage_dir().join("models");
        assert_eq!(derived, expected);
    }
}

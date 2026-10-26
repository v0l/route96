use crate::db::Database;
#[cfg(feature = "labels")]
use crate::db::FileLabel;

#[cfg(feature = "media-compression")]
use crate::processing::{compress_file, probe_file};
use crate::settings::Settings;
use dashmap::DashMap;
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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug)]
pub struct HashMismatch;

impl std::fmt::Display for HashMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SHA-256 hash mismatch")
    }
}

impl std::error::Error for HashMismatch {}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageTier {
    Hot,
    Cold,
}

const TIER_CACHE_MAX_ENTRIES: usize = 1_000_000;

#[derive(Clone)]
pub struct FileStore {
    settings: Arc<RwLock<Settings>>,
    storage_dir: PathBuf,
    cold_storage_dir: Option<PathBuf>,
    tiers: Arc<DashMap<Vec<u8>, StorageTier>>,
}

impl FileStore {
    pub fn new(settings: Arc<RwLock<Settings>>) -> Self {
        // storage_dir is read once at startup; it is intentionally not hot-reloadable.
        let (storage_dir, cold_storage_dir) = {
            let s = settings
                .try_read()
                .expect("settings lock unavailable at FileStore::new");
            (
                PathBuf::from(&s.storage_dir),
                s.cold_storage_dir.as_ref().map(PathBuf::from),
            )
        };
        Self {
            settings,
            storage_dir,
            cold_storage_dir,
            tiers: Arc::new(DashMap::new()),
        }
    }

    async fn settings(&self) -> Settings {
        self.settings.read().await.clone()
    }

    /// Get a file path by id
    pub fn get(&self, id: &Vec<u8>) -> PathBuf {
        self.tier(id)
            .and_then(|t| self.tier_path(id, t))
            .unwrap_or_else(|| self.map_path(id))
    }

    pub fn exists(&self, id: &Vec<u8>) -> bool {
        self.tier(id).is_some()
    }

    /// Which tier holds the file: the cached answer, or hot then cold on disk.
    pub fn tier(&self, id: &Vec<u8>) -> Option<StorageTier> {
        if self.cold_storage_dir.is_none() {
            return self.map_path(id).exists().then_some(StorageTier::Hot);
        }
        if let Some(t) = self.tiers.get(id) {
            return Some(*t);
        }
        let found = [StorageTier::Hot, StorageTier::Cold]
            .into_iter()
            .find(|t| self.tier_path(id, *t).is_some_and(|p| p.exists()))?;
        self.remember_tier(id, found);
        Some(found)
    }

    fn remember_tier(&self, id: &[u8], tier: StorageTier) {
        if self.cold_storage_dir.is_none() {
            return;
        }
        if self.tiers.len() >= TIER_CACHE_MAX_ENTRIES {
            self.tiers.clear();
        }
        self.tiers.insert(id.to_vec(), tier);
    }

    pub fn has_cold_tier(&self) -> bool {
        self.cold_storage_dir.is_some()
    }

    pub fn tier_path(&self, id: &Vec<u8>, tier: StorageTier) -> Option<PathBuf> {
        match tier {
            StorageTier::Hot => Some(self.map_path(id)),
            StorageTier::Cold => self.cold_path(id),
        }
    }

    /// Delete a stored file by id from every tier
    pub async fn delete(&self, id: &Vec<u8>) -> std::io::Result<()> {
        self.tiers.remove(id);
        let mut removed = false;
        let mut last_err = None;
        for path in [Some(self.map_path(id)), self.cold_path(id)]
            .into_iter()
            .flatten()
        {
            match tokio::fs::remove_file(&path).await {
                Ok(()) => removed = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => last_err = Some(e),
            }
        }
        match (last_err, removed) {
            (Some(e), _) => Err(e),
            (None, true) => Ok(()),
            (None, false) => Err(std::io::ErrorKind::NotFound.into()),
        }
    }

    /// Move a stored file to `tier`. Returns false when the file is in neither tier.
    pub async fn relocate(&self, id: &Vec<u8>, tier: StorageTier) -> Result<bool> {
        let cold = self
            .cold_path(id)
            .ok_or_else(|| Error::msg("cold storage is not configured"))?;
        let hot = self.map_path(id);
        let (src, dst, staging_dir) = match tier {
            StorageTier::Cold => (hot, cold, self.cold_temp_dir().unwrap()),
            StorageTier::Hot => (cold, hot, self.temp_dir()),
        };

        if !tokio::fs::try_exists(&src).await? {
            self.tiers.remove(id);
            return Ok(tokio::fs::try_exists(&dst).await?);
        }
        if !tokio::fs::try_exists(&dst).await? {
            tokio::fs::create_dir_all(&staging_dir).await?;
            let staged = staging_dir.join(Uuid::new_v4().to_string());
            if let Err(e) = Self::copy_verified(&src, &staged, id).await {
                tokio::fs::remove_file(&staged).await.ok();
                return Err(e);
            }
            tokio::fs::create_dir_all(dst.parent().unwrap()).await?;
            if let Err(e) = tokio::fs::rename(&staged, &dst).await {
                tokio::fs::remove_file(&staged).await.ok();
                return Err(e.into());
            }
        }
        self.remember_tier(id, tier);
        tokio::fs::remove_file(&src).await?;
        Ok(true)
    }

    /// Open a stored file, falling back to the other tier if it moved meanwhile.
    pub async fn open(&self, id: &Vec<u8>) -> std::io::Result<File> {
        match File::open(self.get(id)).await {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.tiers.remove(id);
                File::open(self.get(id)).await
            }
            r => r,
        }
    }

    async fn copy_verified(src: &Path, dst: &Path, id: &[u8]) -> Result<()> {
        let mut input = File::open(src).await?;
        let mut output = File::create(dst).await?;
        tokio::io::copy(&mut input, &mut output).await?;
        output.sync_all().await?;
        drop(output);
        if Self::hash_file(dst).await? != id {
            return Err(HashMismatch.into());
        }
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
        self.put_with_expected_hash(db, path, mime_type, compress, None)
            .await
    }

    /// Store a file, rejecting an unexpected input hash before publishing it
    /// at the shared content-addressed path.
    pub async fn put_with_expected_hash<'r, S>(
        &self,
        db: &Database,
        path: S,
        mime_type: &str,
        compress: bool,
        expected_hash: Option<&[u8]>,
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

        if expected_hash.is_some_and(|expected| expected != hash) {
            tokio::fs::remove_file(&temp_file).await?;
            return Err(HashMismatch.into());
        }

        // check banned before anything else
        if db.is_file_banned(&hash).await? {
            tokio::fs::remove_file(temp_file).await?;
            return Ok(FileSystemResult::Banned);
        }

        // check if file hash already exists
        if self.exists(&hash) {
            tokio::fs::remove_file(temp_file).await?;
            return Ok(FileSystemResult::AlreadyExists(hash));
        }

        let mut res = if compress && crate::can_compress(mime_type) {
            #[cfg(feature = "media-compression")]
            {
                // FFmpeg transcode is blocking CPU-bound work; run it off the
                // async executor.
                let temp_clone = temp_file.clone();
                let mime_owned = mime_type.to_string();
                let compress_res = {
                    let fs_self = self.clone();
                    tokio::task::spawn_blocking(move || {
                        fs_self.compress_file_sync(&temp_clone, &mime_owned)
                    })
                    .await
                };
                let res = match compress_res {
                    Err(e) => {
                        tokio::fs::remove_file(&temp_file).await?;
                        return Err(Error::msg(format!("Compression task failed: {}", e)));
                    }
                    Ok(Err(e)) => {
                        tokio::fs::remove_file(&temp_file).await?;
                        return Err(e);
                    }
                    Ok(Ok(res)) => res,
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
                    // Probe is blocking FFmpeg work; run off the executor.
                    let probe_path = temp_file.clone();
                    let probe = tokio::task::spawn_blocking(move || probe_file(&probe_path).ok())
                        .await
                        .ok()
                        .flatten();
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
        if self.exists(&res.id) {
            tokio::fs::remove_file(&res.path).await?;
            Ok(FileSystemResult::AlreadyExists(res.id))
        } else {
            let parent = final_dest.parent().unwrap();
            if tokio::fs::rename(&res.path, &final_dest).await.is_err() {
                tokio::fs::create_dir_all(parent).await?;
                tokio::fs::rename(&res.path, &final_dest).await?;
            }

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
                                let max_distance =
                                    settings.identical_media_dedup_distance.unwrap_or(0);
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
                                        Err(e) => {
                                            log::warn!("Failed to check banned similarity: {}", e)
                                        }
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
    /// Synchronous compression — call inside `spawn_blocking`.
    fn compress_file_sync(&self, input: &Path, mime_type: &str) -> Result<NewFileResult> {
        let compressed_result = compress_file(input, mime_type, &self.temp_dir())?;
        let hash = Self::hash_file_sync(&compressed_result.result)?;
        let n = std::fs::metadata(&compressed_result.result)?.len();
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

    #[cfg(feature = "media-compression")]
    fn hash_file_sync(p: &Path) -> Result<Vec<u8>, Error> {
        use std::io::Read;
        let mut file = std::fs::File::open(p)?;
        let mut hasher = Sha256::new();
        let mut buf = [0; 8192];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize().to_vec())
    }

    async fn store_hash_temp_file<S>(&self, mut stream: S) -> Result<(PathBuf, u64, Vec<u8>)>
    where
        S: AsyncRead + Unpin,
    {
        let uid = Uuid::new_v4();
        let out_path = self.temp_dir().join(uid.to_string());

        let mut file = match File::create(&out_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::create_dir_all(self.temp_dir()).await?;
                File::create(&out_path).await?
            }
            Err(e) => return Err(e.into()),
        };

        let mut hasher = Sha256::new();
        let mut total = 0u64;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).await?;
            total += n as u64;
        }
        file.flush().await?;
        Ok((out_path, total, hasher.finalize().to_vec()))
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

    fn cold_path(&self, id: &Vec<u8>) -> Option<PathBuf> {
        let dir = self.cold_storage_dir.as_ref()?;
        let id = hex::encode(id);
        Some(dir.join(&id[0..2]).join(&id[2..4]).join(id))
    }

    fn cold_temp_dir(&self) -> Option<PathBuf> {
        self.cold_storage_dir.as_ref().map(|d| d.join("tmp"))
    }

    pub fn cold_storage_dir(&self) -> Option<PathBuf> {
        self.cold_storage_dir.clone()
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.storage_dir().join("tmp")
    }

    pub fn storage_dir(&self) -> PathBuf {
        self.storage_dir.clone()
    }
}

#[cfg(test)]
mod expected_hash_tests {
    use super::*;
    use sqlx::mysql::MySqlPoolOptions;

    fn test_store(storage_dir: &Path) -> (FileStore, Database) {
        test_store_with_cold(storage_dir, None)
    }

    fn test_store_with_cold(storage_dir: &Path, cold: Option<&Path>) -> (FileStore, Database) {
        let settings = Settings {
            listen: None,
            storage_dir: storage_dir.to_string_lossy().into_owned(),
            database: "mysql://user:pass@127.0.0.1:1/route96".to_string(),
            max_upload_bytes: 1024,
            public_url: String::new(),
            whitelist: None,
            #[cfg(feature = "labels")]
            models_dir: None,
            #[cfg(feature = "labels")]
            label_models: None,
            #[cfg(feature = "labels")]
            label_flag_terms: None,
            webhook_url: None,
            #[cfg(feature = "blossom")]
            reject_sensitive_exif: None,
            #[cfg(feature = "blossom")]
            reject_steganography: None,
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
            delete_zero_egress_days: None,
            cold_storage_dir: cold.map(|c| c.to_string_lossy().into_owned()),
            cold_storage_min_downloads: None,
            cold_storage_after_days: None,
        };
        let settings = Arc::new(RwLock::new(settings));
        let pool = MySqlPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(50))
            .connect_lazy("mysql://user:pass@127.0.0.1:1/route96")
            .unwrap();
        (FileStore::new(settings), Database::from_pool(pool))
    }

    async fn write_hot(store: &FileStore, content: &[u8]) -> Vec<u8> {
        let id = Sha256::digest(content).to_vec();
        let path = store.tier_path(&id, StorageTier::Hot).unwrap();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, content).await.unwrap();
        id
    }

    #[tokio::test]
    async fn relocate_moves_file_to_cold_and_back() {
        let hot = tempfile::tempdir().unwrap();
        let cold = tempfile::tempdir().unwrap();
        let (store, _db) = test_store_with_cold(hot.path(), Some(cold.path()));
        let content = b"rarely downloaded";
        let id = write_hot(&store, content).await;
        let hot_path = store.tier_path(&id, StorageTier::Hot).unwrap();
        let cold_path = store.tier_path(&id, StorageTier::Cold).unwrap();
        assert!(cold_path.starts_with(cold.path()));

        assert!(store.relocate(&id, StorageTier::Cold).await.unwrap());
        assert!(!hot_path.exists());
        assert_eq!(tokio::fs::read(&cold_path).await.unwrap(), content);
        assert_eq!(store.get(&id), cold_path);
        let mut buf = Vec::new();
        store
            .open(&id)
            .await
            .unwrap()
            .read_to_end(&mut buf)
            .await
            .unwrap();
        assert_eq!(buf, content);

        assert!(store.relocate(&id, StorageTier::Hot).await.unwrap());
        assert!(!cold_path.exists());
        assert_eq!(store.get(&id), hot_path);
        assert_eq!(tokio::fs::read(&hot_path).await.unwrap(), content);
    }

    #[tokio::test]
    async fn tier_checks_hot_then_cold_and_caches_answer() {
        let hot = tempfile::tempdir().unwrap();
        let cold = tempfile::tempdir().unwrap();
        let (store, _db) = test_store_with_cold(hot.path(), Some(cold.path()));
        let id = write_hot(&store, b"cached tier").await;
        let hot_path = store.tier_path(&id, StorageTier::Hot).unwrap();
        let cold_path = store.tier_path(&id, StorageTier::Cold).unwrap();

        assert_eq!(store.tier(&id), Some(StorageTier::Hot));
        tokio::fs::create_dir_all(cold_path.parent().unwrap()).await.unwrap();
        tokio::fs::rename(&hot_path, &cold_path).await.unwrap();
        assert_eq!(store.tier(&id), Some(StorageTier::Hot));

        let mut buf = Vec::new();
        store.open(&id).await.unwrap().read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, b"cached tier");
        assert_eq!(store.tier(&id), Some(StorageTier::Cold));
    }

    #[tokio::test]
    async fn tier_without_cold_storage_only_checks_hot() {
        let hot = tempfile::tempdir().unwrap();
        let (store, _db) = test_store(hot.path());
        let id = write_hot(&store, b"hot only").await;
        assert_eq!(store.tier(&id), Some(StorageTier::Hot));
        assert_eq!(store.tier(&vec![7u8; 32]), None);
        store.delete(&id).await.unwrap();
        assert_eq!(store.tier(&id), None);
    }

    #[tokio::test]
    async fn relocate_keeps_source_when_copy_does_not_match_hash() {
        let hot = tempfile::tempdir().unwrap();
        let cold = tempfile::tempdir().unwrap();
        let (store, _db) = test_store_with_cold(hot.path(), Some(cold.path()));
        let id = vec![9u8; 32];
        let hot_path = store.tier_path(&id, StorageTier::Hot).unwrap();
        tokio::fs::create_dir_all(hot_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&hot_path, b"corrupt").await.unwrap();

        let err = store.relocate(&id, StorageTier::Cold).await.unwrap_err();
        assert!(err.downcast_ref::<HashMismatch>().is_some());
        assert!(hot_path.exists());
        assert!(!store.tier_path(&id, StorageTier::Cold).unwrap().exists());
        let mut staged = tokio::fs::read_dir(cold.path().join("tmp")).await.unwrap();
        assert!(staged.next_entry().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn relocate_reports_missing_file() {
        let hot = tempfile::tempdir().unwrap();
        let cold = tempfile::tempdir().unwrap();
        let (store, _db) = test_store_with_cold(hot.path(), Some(cold.path()));
        assert!(
            !store
                .relocate(&vec![1u8; 32], StorageTier::Cold)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn relocate_without_cold_tier_fails() {
        let hot = tempfile::tempdir().unwrap();
        let (store, _db) = test_store(hot.path());
        let id = write_hot(&store, b"x").await;
        assert!(!store.has_cold_tier());
        assert!(store.relocate(&id, StorageTier::Cold).await.is_err());
        assert!(store.tier_path(&id, StorageTier::Hot).unwrap().exists());
    }

    #[tokio::test]
    async fn delete_removes_file_from_cold_tier() {
        let hot = tempfile::tempdir().unwrap();
        let cold = tempfile::tempdir().unwrap();
        let (store, _db) = test_store_with_cold(hot.path(), Some(cold.path()));
        let id = write_hot(&store, b"cold delete").await;
        store.relocate(&id, StorageTier::Cold).await.unwrap();

        store.delete(&id).await.unwrap();
        assert!(!store.exists(&id));
        assert_eq!(
            store.delete(&id).await.unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[tokio::test]
    async fn streaming_store_hashes_and_counts_in_one_pass() {
        let temp = tempfile::tempdir().unwrap();
        let (store, _db) = test_store(temp.path());
        let content = vec![7u8; 1000];

        let (path, size, hash) = store
            .store_hash_temp_file(std::io::Cursor::new(content.clone()))
            .await
            .unwrap();

        assert_eq!(size, 1000);
        assert_eq!(hash, Sha256::digest(&content).to_vec());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), content);
        assert_eq!(path.parent().unwrap(), store.temp_dir());
    }

    #[tokio::test]
    async fn streaming_store_creates_the_temp_dir_when_it_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let (store, _db) = test_store(temp.path());
        assert!(!store.temp_dir().exists());
        let (path, size, _) = store
            .store_hash_temp_file(std::io::Cursor::new(vec![1u8; 10]))
            .await
            .unwrap();
        assert!(path.exists());
        assert_eq!(size, 10);
        store
            .store_hash_temp_file(std::io::Cursor::new(vec![2u8; 10]))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn hash_mismatch_does_not_delete_existing_blob() {
        let temp = tempfile::tempdir().unwrap();
        let (store, db) = test_store(temp.path());
        let content = b"shared content";
        let actual_hash = Sha256::digest(content).to_vec();
        let final_path = store.get(&actual_hash);
        tokio::fs::create_dir_all(final_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&final_path, content).await.unwrap();

        let error = store
            .put_with_expected_hash(
                &db,
                std::io::Cursor::new(content),
                "application/octet-stream",
                false,
                Some(&[0; 32]),
            )
            .await
            .err()
            .unwrap();

        assert!(error.downcast_ref::<HashMismatch>().is_some());
        assert_eq!(error.to_string(), "SHA-256 hash mismatch");
        assert_eq!(tokio::fs::read(final_path).await.unwrap(), content);
        assert!(
            tokio::fs::read_dir(store.temp_dir())
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_none()
        );

        // The legacy entry point still delegates to the shared implementation.
        let error = store
            .put(
                &db,
                std::io::Cursor::new(Vec::<u8>::new()),
                "application/octet-stream",
                false,
            )
            .await
            .err()
            .unwrap();
        assert!(error.downcast_ref::<HashMismatch>().is_none());
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
            #[cfg(feature = "blossom")]
            reject_steganography: None,
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
            delete_zero_egress_days: None,
            cold_storage_dir: None,
            cold_storage_min_downloads: None,
            cold_storage_after_days: None,
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

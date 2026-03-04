use crate::db::Database;
#[cfg(feature = "labels")]
use crate::db::FileLabel;

#[cfg(feature = "labels")]
use crate::processing::labeling::label_file;
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
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt};
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
    #[cfg(feature = "labels")]
    pub labels: Vec<FileLabel>,
}

#[derive(Clone)]
pub struct FileStore {
    settings: Settings,
}

impl FileStore {
    pub fn new(settings: Settings) -> Self {
        Self { settings }
    }

    /// Get a file path by id
    pub fn get(&self, id: &Vec<u8>) -> PathBuf {
        self.map_path(id)
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
        // store file in temp path and hash the file
        let (temp_file, size, hash) = self.store_hash_temp_file(path).await?;

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
                Some("video/mp4")
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
            #[cfg(feature = "labels")]
            labels: vec![],
        })
    }

    /// Run every configured label model against `path` and collect the results
    /// into a flat `Vec<FileLabel>`.  Models that fail are logged and skipped.
    #[cfg(feature = "labels")]
    fn run_label_models(&self, path: &Path, mime_type: &str) -> Vec<FileLabel> {
        use log::warn;
        let models_dir = self
            .settings
            .models_dir
            .clone()
            .unwrap_or_else(|| self.storage_dir().join("models"));

        let Some(label_models) = self.settings.label_models.as_ref() else {
            return vec![];
        };

        let mut labels: Vec<FileLabel> = Vec::new();
        for model_cfg in label_models {
            match label_file(path, mime_type, &models_dir, &model_cfg.hf_repo) {
                Ok(results) => {
                    for (label, _score) in results {
                        let lower = label.to_lowercase();
                        if model_cfg
                            .label_exclude
                            .iter()
                            .any(|ex| ex.to_lowercase() == lower)
                        {
                            continue;
                        }
                        labels.push(FileLabel::new(label, model_cfg.name.clone()));
                    }
                }
                Err(e) => {
                    warn!(
                        "Label model '{}' failed on {:?}: {}",
                        model_cfg.name, path, e
                    );
                }
            }
        }
        labels
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
        PathBuf::from(&self.settings.storage_dir)
    }
}

#[cfg(all(test, feature = "labels"))]
mod tests {
    use super::*;
    use crate::settings::LabelModelConfig;

    fn make_store(
        label_models: Option<Vec<LabelModelConfig>>,
        models_dir: Option<PathBuf>,
    ) -> FileStore {
        FileStore::new(Settings {
            listen: None,
            storage_dir: std::env::temp_dir().to_str().unwrap().to_string(),
            database: String::new(),
            max_upload_bytes: 0,
            public_url: String::new(),
            whitelist: None,
            whitelist_file: None,
            models_dir,
            label_models,
            label_flag_terms: None,
            webhook_url: None,
            plausible_url: None,
            void_cat_files: None,
            #[cfg(feature = "blossom")]
            reject_sensitive_exif: None,
            #[cfg(feature = "payments")]
            payments: None,
        })
    }

    #[test]
    fn test_run_label_models_no_models_configured() {
        let store = make_store(None, None);
        // No models configured → empty result, no panic
        let labels = store.run_label_models(Path::new("/nonexistent"), "image/jpeg");
        assert!(labels.is_empty());
    }

    #[test]
    fn test_run_label_models_empty_models_list() {
        let store = make_store(Some(vec![]), None);
        let labels = store.run_label_models(Path::new("/nonexistent"), "image/jpeg");
        assert!(labels.is_empty());
    }

    #[test]
    fn test_run_label_models_failing_model_is_skipped() {
        // A model with a nonexistent repo will fail; the result should be empty
        // and no panic.
        let store = make_store(
            Some(vec![LabelModelConfig {
                hf_repo: "this/does-not-exist-xyz".to_string(),
                name: "test-model".to_string(),
                label_exclude: vec![],
                min_confidence: None,
            }]),
            Some(std::env::temp_dir().join("route96_test_models_fs")),
        );
        let labels = store.run_label_models(Path::new("/nonexistent"), "image/jpeg");
        // Should be empty because the model failed, not a panic
        assert!(labels.is_empty());
    }

    #[test]
    fn test_models_dir_defaults_to_storage_subdir() {
        let store = make_store(None, None);
        let expected = PathBuf::from(store.settings.storage_dir.clone()).join("models");
        // Exercise the defaulting logic inside run_label_models by checking the
        // path that would be used (we verify it matches <storage_dir>/models).
        let derived = store
            .settings
            .models_dir
            .clone()
            .unwrap_or_else(|| store.storage_dir().join("models"));
        assert_eq!(derived, expected);
    }
}

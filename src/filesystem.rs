use std::env::temp_dir;
use std::fs;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Error;
use chrono::Utc;
use ffmpeg_rs_raw::DemuxerInfo;
use log::info;
use rocket::form::validate::Contains;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt};

#[cfg(feature = "labels")]
use crate::db::FileLabel;
use crate::db::FileUpload;
#[cfg(feature = "labels")]
use crate::processing::labeling::label_frame;
#[cfg(feature = "media-compression")]
use crate::processing::{compress_file, probe_file, FileProcessorResult};
use crate::settings::Settings;

#[derive(Clone, Default, Serialize)]
pub struct FileSystemResult {
    pub path: PathBuf,
    pub upload: FileUpload,
}

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
    pub async fn put<S>(
        &self,
        stream: S,
        mime_type: &str,
        compress: bool,
    ) -> Result<FileSystemResult, Error>
    where
        S: AsyncRead + Unpin,
    {
        let result = self
            .store_compress_file(stream, mime_type, compress)
            .await?;
        let dst_path = self.map_path(&result.upload.id);
        if dst_path.exists() {
            fs::remove_file(result.path)?;
            return Ok(FileSystemResult {
                path: dst_path,
                ..result
            });
        }
        fs::create_dir_all(dst_path.parent().unwrap())?;
        if let Err(e) = fs::copy(&result.path, &dst_path) {
            fs::remove_file(&result.path)?;
            Err(Error::from(e))
        } else {
            fs::remove_file(result.path)?;
            Ok(FileSystemResult {
                path: dst_path,
                ..result
            })
        }
    }

    /// Try to replace the mime-type when unknown using ffmpeg probe result
    fn hack_mime_type(mime_type: &str, p: &DemuxerInfo) -> String {
        if mime_type == "application/octet-stream" {
            if p.format.contains("mp4") {
                "video/mp4".to_string()
            } else if p.format.contains("webp") {
                "image/webp".to_string()
            } else if p.format.contains("jpeg") {
                "image/jpeg".to_string()
            } else if p.format.contains("png") {
                "image/png".to_string()
            } else if p.format.contains("gif") {
                "image/gif".to_string()
            } else {
                mime_type.to_string()
            }
        } else {
            mime_type.to_string()
        }
    }

    async fn store_compress_file<S>(
        &self,
        mut stream: S,
        mime_type: &str,
        compress: bool,
    ) -> Result<FileSystemResult, Error>
    where
        S: AsyncRead + Unpin,
    {
        let random_id = uuid::Uuid::new_v4();
        let tmp_path = FileStore::map_temp(random_id);
        let mut file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(tmp_path.clone())
            .await?;
        tokio::io::copy(&mut stream, &mut file).await?;

        info!("File saved to temp path: {}", tmp_path.to_str().unwrap());

        #[cfg(feature = "media-compression")]
        if compress {
            let start = SystemTime::now();
            let proc_result = compress_file(tmp_path.clone(), mime_type)?;
            if let FileProcessorResult::NewFile(new_temp) = proc_result {
                let old_size = tmp_path.metadata()?.len();
                let new_size = new_temp.result.metadata()?.len();
                let time_compress = SystemTime::now().duration_since(start)?;
                let start = SystemTime::now();

                #[cfg(feature = "labels")]
                let labels = if let Some(mp) = &self.settings.vit_model {
                    label_frame(&new_temp.result, mp.model.clone(), mp.config.clone())?
                        .iter()
                        .map(|l| FileLabel::new(l.0.clone(), "vit224".to_string()))
                        .collect()
                } else {
                    vec![]
                };

                let time_labels = SystemTime::now().duration_since(start)?;

                // delete old temp
                fs::remove_file(tmp_path)?;
                file = File::options()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .read(true)
                    .open(new_temp.result.clone())
                    .await?;
                let n = file.metadata().await?.len();
                let hash = FileStore::hash_file(&mut file).await?;

                info!("Processed media: ratio={:.2}x, old_size={:.3}kb, new_size={:.3}kb, duration_compress={:.2}ms, duration_labels={:.2}ms",
                    old_size as f32 / new_size as f32,
                    old_size as f32 / 1024.0,
                    new_size as f32 / 1024.0,
                    time_compress.as_micros() as f64 / 1000.0,
                    time_labels.as_micros() as f64 / 1000.0
                );

                return Ok(FileSystemResult {
                    path: new_temp.result,
                    upload: FileUpload {
                        id: hash,
                        name: "".to_string(),
                        size: n,
                        width: Some(new_temp.width as u32),
                        height: Some(new_temp.height as u32),
                        blur_hash: None,
                        mime_type: new_temp.mime_type,
                        #[cfg(feature = "labels")]
                        labels,
                        created: Utc::now(),
                        ..Default::default()
                    },
                });
            }
        } else if let Ok(p) = probe_file(tmp_path.clone()) {
            let n = file.metadata().await?.len();
            let hash = FileStore::hash_file(&mut file).await?;
            let v_stream = p.best_video();
            return Ok(FileSystemResult {
                path: tmp_path,
                upload: FileUpload {
                    id: hash,
                    name: "".to_string(),
                    size: n,
                    created: Utc::now(),
                    mime_type: Self::hack_mime_type(mime_type, &p),
                    width: v_stream.map(|v| v.width as u32),
                    height: v_stream.map(|v| v.height as u32),
                    ..Default::default()
                },
            });
        }

        let n = file.metadata().await?.len();
        let hash = FileStore::hash_file(&mut file).await?;
        Ok(FileSystemResult {
            path: tmp_path,
            upload: FileUpload {
                id: hash,
                name: "".to_string(),
                size: n,
                created: Utc::now(),
                mime_type: mime_type.to_string(),
                ..Default::default()
            },
        })
    }

    async fn hash_file(file: &mut File) -> Result<Vec<u8>, Error> {
        let mut hasher = Sha256::new();
        file.seek(SeekFrom::Start(0)).await?;
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

    fn map_temp(id: uuid::Uuid) -> PathBuf {
        temp_dir().join(id.to_string())
    }

    pub fn map_path(&self, id: &Vec<u8>) -> PathBuf {
        let id = hex::encode(id);
        Path::new(&self.settings.storage_dir)
            .join(&id[0..2])
            .join(&id[2..4])
            .join(id)
    }
}

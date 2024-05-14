use std::env::temp_dir;
use std::fs;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::Error;
use log::info;
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt};

use crate::processing::{compress_file, FileProcessorResult};
use crate::processing::labeling::label_frame;
use crate::settings::Settings;

#[derive(Clone, Default)]
pub struct FileSystemResult {
    pub path: PathBuf,
    pub sha256: Vec<u8>,
    pub size: u64,
    pub mime_type: String,
    pub width: Option<usize>,
    pub height: Option<usize>,
    pub blur_hash: Option<String>,
    pub labels: Option<Vec<String>>,
}

pub struct FileStore {
    settings: Settings,
}

impl FileStore {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
        }
    }

    /// Get a file path by id
    pub fn get(&self, id: &Vec<u8>) -> PathBuf {
        self.map_path(id)
    }

    /// Store a new file
    pub async fn put<TStream>(&self, stream: TStream, mime_type: &str, compress: bool) -> Result<FileSystemResult, Error>
        where
            TStream: AsyncRead + Unpin,
    {
        let result = self.store_compress_file(stream, mime_type, compress).await?;
        let dst_path = self.map_path(&result.sha256);
        if dst_path.exists() {
            fs::remove_file(&result.path)?;
            return Err(Error::msg("File already exists"));
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

    async fn store_compress_file<TStream>(&self, mut stream: TStream, mime_type: &str, compress: bool) -> Result<FileSystemResult, Error>
        where
            TStream: AsyncRead + Unpin,
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

        if compress {
            let start = SystemTime::now();
            let proc_result = compress_file(tmp_path.clone(), mime_type)?;
            if let FileProcessorResult::NewFile(mut new_temp) = proc_result {
                let old_size = tmp_path.metadata()?.len();
                let new_size = new_temp.result.metadata()?.len();
                let time_compress = SystemTime::now().duration_since(start).unwrap();
                let start = SystemTime::now();
                let blur_hash = blurhash::encode(
                    9, 9,
                    new_temp.width as u32,
                    new_temp.height as u32,
                    new_temp.image.as_slice(),
                )?;
                let time_blurhash = SystemTime::now().duration_since(start).unwrap();
                let start = SystemTime::now();
                let labels = if let Some(mp) = &self.settings.vit_model_path {
                    label_frame(
                        new_temp.image.as_mut_slice(),
                        new_temp.width,
                        new_temp.height,
                        mp.clone())?
                } else {
                    vec![]
                };
                let time_labels = SystemTime::now().duration_since(start).unwrap();

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

                info!("Processed media: ratio={:.2}x, old_size={:.3}kb, new_size={:.3}kb, duration_compress={:.2}ms, duration_blurhash={:.2}ms, duration_labels={:.2}ms",
                    old_size as f32 / new_size as f32,
                    old_size as f32 / 1024.0,
                    new_size as f32 / 1024.0,
                    time_compress.as_micros() as f64 / 1000.0,
                    time_blurhash.as_micros() as f64 / 1000.0,
                    time_labels.as_micros() as f64 / 1000.0
                );

                return Ok(FileSystemResult {
                    size: n,
                    sha256: hash,
                    path: new_temp.result,
                    width: Some(new_temp.width),
                    height: Some(new_temp.height),
                    blur_hash: Some(blur_hash),
                    mime_type: new_temp.mime_type,
                    labels: Some(labels),
                });
            }
        }
        let n = file.metadata().await?.len();
        let hash = FileStore::hash_file(&mut file).await?;
        Ok(FileSystemResult {
            path: tmp_path,
            sha256: hash,
            size: n,
            mime_type: mime_type.to_string(),
            ..Default::default()
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

    fn map_path(&self, id: &Vec<u8>) -> PathBuf {
        let id = hex::encode(id);
        Path::new(&self.settings.storage_dir)
            .join(&id[0..2])
            .join(&id[2..4])
            .join(id)
    }
}

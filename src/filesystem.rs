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

use crate::processing::{FileProcessor, MediaProcessor};
use crate::settings::Settings;

#[derive(Clone)]
pub struct FileSystemResult {
    pub path: PathBuf,
    pub sha256: Vec<u8>,
    pub size: u64,
}

pub struct FileStore {
    path: String,
    processor: Arc<Mutex<MediaProcessor>>,
}

impl FileStore {
    pub fn new(settings: Settings) -> Self {
        Self {
            path: settings.storage_dir,
            processor: Arc::new(Mutex::new(MediaProcessor::new())),
        }
    }

    /// Get a file path by id
    pub fn get(&self, id: &Vec<u8>) -> PathBuf {
        self.map_path(id)
    }

    /// Store a new file
    pub async fn put<TStream>(&self, mut stream: TStream, mime_type: &str) -> Result<FileSystemResult, Error>
        where
            TStream: AsyncRead + Unpin,
    {
        let random_id = uuid::Uuid::new_v4();

        let (n, hash, tmp_path) = {
            let tmp_path = FileStore::map_temp(random_id);
            let mut file = File::options()
                .create(true)
                .write(true)
                .read(true)
                .open(tmp_path.clone())
                .await?;
            tokio::io::copy(&mut stream, &mut file).await?;

            info!("File saved to temp path: {}", tmp_path.to_str().unwrap());

            let start = SystemTime::now();
            let new_temp = {
                let mut p_lock = self.processor.lock().expect("asd");
                p_lock.process_file(tmp_path.clone(), mime_type)?
            };
            let old_size = tmp_path.metadata()?.len();
            let new_size = new_temp.metadata()?.len();
            info!("Compressed media: ratio={:.2}x, old_size={:.3}kb, new_size={:.3}kb, duration={:.2}ms",
                old_size as f32 / new_size as f32,
                old_size as f32 / 1024.0,
                new_size as f32 / 1024.0,
                SystemTime::now().duration_since(start).unwrap().as_micros() as f64 / 1000.0
            );

            let mut file = File::options()
                .create(true)
                .write(true)
                .read(true)
                .open(new_temp.clone())
                .await?;
            let n = file.metadata().await?.len();
            let hash = FileStore::hash_file(&mut file).await?;
            fs::remove_file(tmp_path)?;
            (n, hash, new_temp)
        };
        let dst_path = self.map_path(&hash);
        fs::create_dir_all(dst_path.parent().unwrap())?;
        if let Err(e) = fs::copy(&tmp_path, &dst_path) {
            fs::remove_file(&tmp_path)?;
            Err(Error::from(e))
        } else {
            fs::remove_file(tmp_path)?;
            Ok(FileSystemResult {
                size: n,
                sha256: hash,
                path: dst_path,
            })
        }
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
        Path::new(&self.path)
            .join(id[0..2].to_string())
            .join(id[2..4].to_string())
            .join(id)
    }
}

use std::env::temp_dir;
use std::fs;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use anyhow::Error;
use log::info;
use rocket::data::DataStream;
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufWriter};

use crate::db::Database;
use crate::settings::Settings;

#[derive(Clone)]
pub struct FileSystemResult {
    pub path: PathBuf,
    pub sha256: Vec<u8>,
    pub size: u64,
}

pub struct FileStore {
    path: String,
}

impl FileStore {
    pub fn new(settings: Settings) -> Self {
        Self {
            path: settings.storage_dir,
        }
    }

    /// Get a file path by id
    pub fn get(&self, id: &Vec<u8>) -> PathBuf {
        self.map_path(id)
    }

    /// Store a new file
    pub async fn put(&self, stream: DataStream<'_>) -> Result<FileSystemResult, Error> {
        let random_id = uuid::Uuid::new_v4();
        let tmp_path = FileStore::map_temp(random_id);

        let mut file = File::options()
            .create(true).write(true).read(true)
            .open(tmp_path.clone()).await?;
        let n = stream.stream_to(&mut BufWriter::new(&mut file)).await?;

        info!("File saved to temp path: {}", tmp_path.to_str().unwrap());
        let hash = FileStore::hash_file(&mut file).await?;
        let dst_path = self.map_path(&hash);
        fs::create_dir_all(dst_path.parent().unwrap())?;
        if let Err(e) = fs::rename(&tmp_path, &dst_path) {
            fs::remove_file(&tmp_path)?;
            Err(Error::from(e))
        } else {
            Ok(FileSystemResult {
                size: n.written,
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

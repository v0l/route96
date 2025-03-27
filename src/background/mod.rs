use crate::db::Database;
use crate::filesystem::FileStore;
use anyhow::Result;
use tokio::task::JoinHandle;

#[cfg(feature = "media-compression")]
mod media_metadata;

pub async fn start_background_tasks(_db: Database, _fs: FileStore) -> Vec<JoinHandle<Result<()>>> {
    let ret = vec![];

    #[cfg(feature = "media-compression")]
    {
        ret.push(tokio::spawn(async move {
            log::info!("Starting MediaMetadata background task");
            let mut m = media_metadata::MediaMetadata::new(_db.clone(), _fs.clone());
            m.process().await?;
            log::info!("MediaMetadata background task completed");
            Ok(())
        }));
    }
    ret
}

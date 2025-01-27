use crate::db::Database;
use crate::filesystem::FileStore;
use anyhow::Result;
use log::info;
use tokio::task::JoinHandle;

#[cfg(feature = "media-compression")]
mod media_metadata;

pub fn start_background_tasks(db: Database, file_store: FileStore) -> Vec<JoinHandle<Result<()>>> {
    let mut ret = vec![];

    #[cfg(feature = "media-compression")]
    {
        ret.push(tokio::spawn(async move {
            info!("Starting MediaMetadata background task");
            let mut m = media_metadata::MediaMetadata::new(db.clone(), file_store.clone());
            m.process().await?;
            info!("MediaMetadata background task completed");
            Ok(())
        }));
    }
    ret
}

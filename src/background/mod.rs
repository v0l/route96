use crate::db::Database;
use crate::filesystem::FileStore;
use anyhow::Result;
use log::{info, warn};
use tokio::task::JoinHandle;

#[cfg(feature = "media-compression")]
mod media_metadata;

#[cfg(feature = "payments")]
mod payments;

pub fn start_background_tasks(
    db: Database,
    file_store: FileStore,
    #[cfg(feature = "payments")] client: Option<fedimint_tonic_lnd::Client>,
) -> Vec<JoinHandle<Result<()>>> {
    let mut ret = vec![];

    #[cfg(feature = "media-compression")]
    {
        let db = db.clone();
        ret.push(tokio::spawn(async move {
            info!("Starting MediaMetadata background task");
            let mut m = media_metadata::MediaMetadata::new(db, file_store.clone());
            m.process().await?;
            info!("MediaMetadata background task completed");
            Ok(())
        }));
    }
    #[cfg(feature = "payments")]
    {
        if let Some(client) = client {
            let db = db.clone();
            ret.push(tokio::spawn(async move {
                info!("Starting PaymentsHandler background task");
                let mut m = payments::PaymentsHandler::new(client, db);
                m.process().await?;
                info!("PaymentsHandler background task completed");
                Ok(())
            }));
        } else {
            warn!("Not starting PaymentsHandler, configuration missing")
        }
    }
    ret
}

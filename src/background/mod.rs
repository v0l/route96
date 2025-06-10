use crate::db::Database;
use crate::filesystem::FileStore;
use log::{error, info, warn};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

#[cfg(feature = "media-compression")]
mod media_metadata;

#[cfg(feature = "payments")]
mod payments;

pub fn start_background_tasks(
    db: Database,
    file_store: FileStore,
    shutdown_rx: broadcast::Receiver<()>,
    #[cfg(feature = "payments")] client: Option<fedimint_tonic_lnd::Client>,
) -> Vec<JoinHandle<()>> {
    let mut ret = vec![];

    #[cfg(feature = "media-compression")]
    {
        let db = db.clone();
        let rx = shutdown_rx.resubscribe();
        ret.push(tokio::spawn(async move {
            info!("Starting MediaMetadata background task");
            let mut m = media_metadata::MediaMetadata::new(db, file_store.clone());
            if let Err(e) = m.process(rx).await {
                error!("MediaMetadata failed: {}", e);
            } else {
                info!("MediaMetadata background task completed");
            }
        }));
    }
    #[cfg(feature = "payments")]
    {
        if let Some(client) = client {
            let db = db.clone();
            let rx = shutdown_rx.resubscribe();
            ret.push(tokio::spawn(async move {
                info!("Starting PaymentsHandler background task");
                let mut m = payments::PaymentsHandler::new(client, db);
                if let Err(e) = m.process(rx).await {
                    error!("PaymentsHandler failed: {}", e);
                } else {
                    info!("PaymentsHandler background task completed");
                }
            }));
        } else {
            warn!("Not starting PaymentsHandler, configuration missing")
        }
    }
    ret
}

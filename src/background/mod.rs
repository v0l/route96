use crate::db::Database;
use crate::filesystem::FileStore;
use crate::settings::Settings;
use log::{error, info};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "media-compression")]
mod media_metadata;

#[cfg(feature = "media-compression")]
mod phash_files;

#[cfg(feature = "labels")]
mod label_files;

#[cfg(feature = "payments")]
mod payments;

pub fn start_background_tasks(
    db: Database,
    file_store: FileStore,
    settings: Settings,
    shutdown: CancellationToken,
    #[cfg(feature = "payments")] client: Option<fedimint_tonic_lnd::Client>,
) -> Vec<JoinHandle<()>> {
    let mut ret = vec![];

    #[cfg(feature = "media-compression")]
    {
        let db = db.clone();
        let fs = file_store.clone();
        let token = shutdown.clone();
        ret.push(tokio::spawn(async move {
            info!("Starting MediaMetadata background task");
            let mut m = media_metadata::MediaMetadata::new(db, fs);
            if let Err(e) = m.process(token).await {
                error!("MediaMetadata failed: {}", e);
            } else {
                info!("MediaMetadata background task completed");
            }
        }));
    }

    #[cfg(feature = "media-compression")]
    {
        let db = db.clone();
        let fs = file_store.clone();
        let token = shutdown.clone();
        ret.push(tokio::spawn(async move {
            info!("Starting PhashFiles background task");
            let task = phash_files::PhashFiles::new(db, fs);
            task.process(token).await;
            info!("PhashFiles background task completed");
        }));
    }

    #[cfg(feature = "labels")]
    {
        if let Some(label_models) = settings.label_models.clone()
            && !label_models.is_empty()
        {
            let db = db.clone();
            let fs = file_store.clone();
            let models_dir = settings
                .models_dir
                .clone()
                .unwrap_or_else(|| fs.storage_dir().join("models"));
            let flag_terms = settings.label_flag_terms.clone().unwrap_or_default();
            let token = shutdown.clone();
            ret.push(tokio::spawn(async move {
                info!("Starting LabelFiles background task");
                let task =
                    label_files::LabelFiles::new(db, fs, models_dir, label_models, flag_terms);
                task.process(token).await;
                info!("LabelFiles background task completed");
            }));
        }
    }

    #[cfg(feature = "payments")]
    {
        if let Some(client) = client {
            let db = db.clone();
            let token = shutdown.clone();
            ret.push(tokio::spawn(async move {
                info!("Starting PaymentsHandler background task");
                let mut m = payments::PaymentsHandler::new(client, db);
                if let Err(e) = m.process(token).await {
                    error!("PaymentsHandler failed: {}", e);
                } else {
                    info!("PaymentsHandler background task completed");
                }
            }));
        } else {
            log::warn!("Not starting PaymentsHandler, configuration missing")
        }
    }

    // Suppress unused-variable warnings when no features are enabled that use `settings`.
    let _ = settings;

    ret
}

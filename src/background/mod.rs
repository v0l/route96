#![allow(unused_variables)] // Allow unused db/storage for now

use crate::db::Database;
use crate::filesystem::FileStore;
use crate::settings::Settings;
use crate::storage::StorageBackend;
use anyhow::Result;
use log::{error, info};
use std::env;
use std::sync::Arc;
use tokio::task::{spawn_blocking, JoinHandle};

#[cfg(feature = "media-compression")]
mod media_metadata;

// Function to start background tasks
pub async fn start_background_tasks(
    db: Arc<Database>,
    storage: Arc<dyn StorageBackend>,
    _settings: &Settings,
) -> Vec<JoinHandle<Result<()>>> {
    let mut handles = Vec::new();

    let temp_dir = env::temp_dir();

    #[cfg(feature = "media-compression")]
    {
        info!("Starting background task to extract media metadata...");
        let db_clone = db.clone();
        let storage_clone = storage.clone();
        let temp_dir_clone = temp_dir.clone();
        let metadata_handle = tokio::spawn(async move {
            if let Some(file_store) = storage_clone.as_any().downcast_ref::<FileStore>() {
                let file_store_arc = Arc::new(file_store.clone());
                // Wrap the potentially non-Send job in spawn_blocking
                spawn_blocking(move || {
                    // Need a sync context here, potentially using `tokio::runtime::Handle::current().block_on()`
                    // if media_metadata::job remains async, or refactor media_metadata::job to be sync.
                    // Assuming media_metadata::job can be called from a blocking context:
                    // If media_metadata::job needs tokio runtime, it must handle it internally.
                    // Let's assume for now it can be driven by block_on or is sync.
                    // We'll simplify by calling it directly if it's sync, or using block_on if async.
                    // If job is async:
                    let handle = tokio::runtime::Handle::current();
                    handle.block_on(media_metadata::job(
                        db_clone,
                        file_store_arc,
                        temp_dir_clone,
                    ))

                    // If job was sync:
                    // media_metadata::job_sync(db_clone, file_store_arc, temp_dir_clone)
                })
                .await??; // First '?' unwraps JoinError, second '?' unwraps the inner Result<()>
                Ok(()) // Explicitly return Ok(()) on success to match the else branch
            } else {
                error!("Media metadata background job only supports FileStore backend.");
                Ok(())
            }
        });
        handles.push(metadata_handle);
    }

    handles
}

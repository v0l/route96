use crate::db::{Database, FileLabel, ReviewState};
use crate::filesystem::FileStore;
use crate::processing::labeling::{MIN_CONFIDENCE, VitModel};
use crate::settings::LabelModelConfig;
use candle_core::Device;
use log::{error, info, warn};
use std::path::PathBuf;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::broadcast;

/// A loaded model paired with its configuration.
struct LoadedModel {
    cfg: LabelModelConfig,
    vit: VitModel,
}

pub struct LabelFiles {
    db: Database,
    fs: FileStore,
    models_dir: PathBuf,
    label_models: Vec<LabelModelConfig>,
    label_flag_terms: Vec<String>,
}

impl LabelFiles {
    pub fn new(
        db: Database,
        fs: FileStore,
        models_dir: PathBuf,
        label_models: Vec<LabelModelConfig>,
        label_flag_terms: Vec<String>,
    ) -> Self {
        Self {
            db,
            fs,
            models_dir,
            label_models,
            label_flag_terms,
        }
    }

    /// Spawn a dedicated OS thread that owns all CUDA state for the lifetime
    /// of the task. All model loading and inference runs on that thread;
    /// async DB calls are driven via the current tokio runtime handle.
    pub async fn process(self, mut shutdown: broadcast::Receiver<()>) {
        let handle = Handle::current();
        // Convert the broadcast receiver into a simple oneshot-style flag via
        // a std channel so the blocking thread can poll it without .await.
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

        // Forward first shutdown signal to the blocking thread.
        tokio::spawn(async move {
            let _ = shutdown.recv().await;
            let _ = stop_tx.send(());
        });

        let thread = std::thread::spawn(move || {
            let device = Device::cuda_if_available(0).unwrap_or_else(|e| {
                error!("Failed to initialize CUDA device: {}", e);
                Device::Cpu
            });

            // Load all models on this thread so the CUDA context is created
            // here and never migrated to another thread.
            let mut models: Vec<LoadedModel> = Vec::new();
            for cfg in &self.label_models {
                info!("Loading label model '{}'", cfg.name);
                match VitModel::load_from_dir(&self.models_dir, &cfg.hf_repo, device.clone()) {
                    Ok(vit) => models.push(LoadedModel {
                        cfg: cfg.clone(),
                        vit,
                    }),
                    Err(e) => {
                        error!("Failed to load label model '{}': {}", cfg.name, e);
                    }
                }
            }

            if models.is_empty() {
                return;
            }

            loop {
                Self::run_batch(&models, &self.db, &self.fs, &self.label_flag_terms, &handle);

                // Sleep 60 s, waking early on shutdown.
                let deadline = std::time::Instant::now() + Duration::from_secs(60);
                loop {
                    if stop_rx.try_recv().is_ok() {
                        return;
                    }
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    std::thread::sleep(remaining.min(Duration::from_millis(200)));
                }
            }
        });

        // Await the thread so the task handle stays alive.
        if let Err(e) = tokio::task::spawn_blocking(move || thread.join()).await {
            error!("LabelFiles thread panicked: {:?}", e);
        }
    }

    fn run_batch(
        models: &[LoadedModel],
        db: &Database,
        fs: &FileStore,
        label_flag_terms: &[String],
        handle: &Handle,
    ) {
        for loaded in models {
            let to_label = match handle.block_on(db.get_files_missing_labels(&loaded.cfg.name)) {
                Ok(v) => v,
                Err(e) => {
                    error!(
                        "Failed to query missing labels for '{}': {}",
                        loaded.cfg.name, e
                    );
                    continue;
                }
            };

            if !to_label.is_empty() {
                info!(
                    "{} files missing labels for model '{}'",
                    to_label.len(),
                    loaded.cfg.name
                );
            }

            for file in to_label {
                let path = fs.get(&file.id);
                if !path.exists() {
                    warn!("Skipping missing file: {}", hex::encode(&file.id));
                    Self::sync_mark_labeled(db, &file.id, &loaded.cfg.name, handle);
                    continue;
                }

                let min_confidence = loaded.cfg.min_confidence.unwrap_or(MIN_CONFIDENCE);

                let new_labels = match loaded.vit.run(&path, &file.mime_type, min_confidence) {
                    Ok(results) => results
                        .into_iter()
                        .filter(|(label, _)| {
                            let lower = label.to_lowercase();
                            !loaded
                                .cfg
                                .label_exclude
                                .iter()
                                .any(|ex| ex.to_lowercase() == lower)
                        })
                        .map(|(label, score)| {
                            info!(
                                "Label: file={} model={} label={} score={:.4}",
                                hex::encode(&file.id),
                                loaded.cfg.name,
                                label,
                                score
                            );
                            FileLabel::new(label, loaded.cfg.name.clone())
                        })
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        error!(
                            "Label model '{}' failed on {}: {}",
                            loaded.cfg.name,
                            hex::encode(&file.id),
                            e
                        );
                        Self::sync_mark_labeled(db, &file.id, &loaded.cfg.name, handle);
                        continue;
                    }
                };

                for label in &new_labels {
                    if let Err(e) = handle.block_on(db.add_file_label(&file.id, label)) {
                        error!(
                            "Failed to save label '{}' for {}: {}",
                            label.label,
                            hex::encode(&file.id),
                            e
                        );
                    }
                }

                Self::sync_mark_labeled(db, &file.id, &loaded.cfg.name, handle);

                if !label_flag_terms.is_empty() && !new_labels.is_empty() {
                    let new_state =
                        Database::review_state_for_labels(&new_labels, label_flag_terms);
                    if new_state != ReviewState::None {
                        if let Err(e) =
                            handle.block_on(db.set_file_review_state(&file.id, new_state))
                        {
                            error!(
                                "Failed to set review state for {}: {}",
                                hex::encode(&file.id),
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    fn sync_mark_labeled(db: &Database, file_id: &[u8], model_name: &str, handle: &Handle) {
        if let Err(e) = handle.block_on(db.add_labeled_by(file_id, model_name)) {
            error!(
                "Failed to update labeled_by for {}: {}",
                hex::encode(file_id),
                e
            );
        }
    }
}

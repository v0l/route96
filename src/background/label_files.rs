use crate::db::{Database, FileLabel, ReviewState};
use crate::filesystem::FileStore;
use crate::processing::labeling::{MediaLabeler, VitLabeler};
use crate::settings::{LabelModelConfig, LabelerType};
use candle_core::Device;
use log::{error, info, warn};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

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

    /// Spawn a dedicated OS thread **per model** so they can process batches
    /// in parallel. Each thread owns its labeler (and any CUDA state it holds).
    pub async fn process(self, shutdown: CancellationToken) {
        let handle = Handle::current();
        let mut threads = Vec::new();

        for cfg in self.label_models.clone() {
            let db = self.db.clone();
            let fs = self.fs.clone();
            let models_dir = self.models_dir.clone();
            let flag_terms = self.label_flag_terms.clone();
            let handle = handle.clone();
            let token = shutdown.clone();

            threads.push(std::thread::spawn(move || {
                let labeler = match Self::build_labeler(&cfg, &models_dir) {
                    Some(l) => l,
                    None => return,
                };

                info!("Label worker '{}' started", labeler.name());

                loop {
                    Self::run_batch(labeler.as_ref(), &db, &fs, &flag_terms, &handle);

                    // Sleep 60s, waking early on shutdown.
                    let deadline = std::time::Instant::now() + Duration::from_secs(60);
                    loop {
                        if token.is_cancelled() {
                            info!("Label worker '{}' shutting down", labeler.name());
                            return;
                        }
                        let remaining =
                            deadline.saturating_duration_since(std::time::Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        std::thread::sleep(remaining.min(Duration::from_millis(200)));
                    }
                }
            }));
        }

        // Await all model threads.
        if let Err(e) = tokio::task::spawn_blocking(move || {
            for t in threads {
                if let Err(e) = t.join() {
                    error!("Label worker thread panicked: {:?}", e);
                }
            }
        })
        .await
        {
            error!("LabelFiles spawn_blocking failed: {:?}", e);
        }
    }

    /// Construct the appropriate [`MediaLabeler`] for a given config entry.
    fn build_labeler(cfg: &LabelModelConfig, models_dir: &Path) -> Option<Box<dyn MediaLabeler>> {
        match &cfg.labeler_type {
            LabelerType::Vit { hf_repo } => {
                info!("Loading ViT label model '{}'", cfg.name);
                let device = Device::cuda_if_available(0).unwrap_or_else(|e| {
                    error!("Failed to initialize CUDA device: {}", e);
                    Device::Cpu
                });
                match VitLabeler::load(
                    models_dir,
                    hf_repo,
                    cfg.name.clone(),
                    cfg.label_exclude.clone(),
                    cfg.min_confidence,
                    device,
                ) {
                    Ok(v) => Some(Box::new(v)),
                    Err(e) => {
                        error!("Failed to load label model '{}': {}", cfg.name, e);
                        None
                    }
                }
            }
        }
    }

    fn run_batch(
        labeler: &dyn MediaLabeler,
        db: &Database,
        fs: &FileStore,
        label_flag_terms: &[String],
        handle: &Handle,
    ) {
        let model_name = labeler.name();
        let to_label = match handle.block_on(db.get_files_missing_labels(model_name)) {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to query missing labels for '{}': {}", model_name, e);
                return;
            }
        };

        if !to_label.is_empty() {
            info!(
                "{} files missing labels for model '{}'",
                to_label.len(),
                model_name
            );
        }

        for file in to_label {
            let path = fs.get(&file.id);
            if !path.exists() {
                warn!("Skipping missing file: {}", hex::encode(&file.id));
                Self::sync_mark_labeled(db, &file.id, model_name, handle);
                continue;
            }

            let start = std::time::Instant::now();
            let new_labels = match labeler.label_file(&path, &file.mime_type) {
                Ok(results) => {
                    let elapsed = start.elapsed();
                    results
                        .into_iter()
                        .filter(|(label, _)| {
                            let lower = label.to_lowercase();
                            !labeler
                                .label_exclude()
                                .iter()
                                .any(|ex| ex.to_lowercase() == lower)
                        })
                        .map(|(label, score)| {
                            info!(
                                "Label: file={} model={} label={} score={:.4} duration={:.2?}",
                                hex::encode(&file.id),
                                model_name,
                                label,
                                score,
                                elapsed,
                            );
                            FileLabel::new(label, model_name.to_string())
                        })
                        .collect::<Vec<_>>()
                }
                Err(e) => {
                    let elapsed = start.elapsed();
                    error!(
                        "Label model '{}' failed on {} after {:.2?}: {}",
                        model_name,
                        hex::encode(&file.id),
                        elapsed,
                        e
                    );
                    Self::sync_mark_labeled(db, &file.id, model_name, handle);
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

            Self::sync_mark_labeled(db, &file.id, model_name, handle);

            if !label_flag_terms.is_empty() && !new_labels.is_empty() {
                let new_state = Database::review_state_for_labels(&new_labels, label_flag_terms);
                if new_state != ReviewState::None {
                    if let Err(e) = handle.block_on(db.set_file_review_state(&file.id, new_state)) {
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

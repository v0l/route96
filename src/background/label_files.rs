use crate::db::{Database, FileLabel, ReviewState};
use crate::filesystem::FileStore;
use crate::processing::labeling::{MediaLabeler, VitLabeler};
use crate::settings::{LabelModelConfig, LabelerType};
use candle_core::Device;
use log::{error, info, warn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
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

    /// Spawn a dedicated tokio task **per model** so they can process batches
    /// in parallel. Each task owns its labeler (and any CUDA state it holds).
    pub async fn process(self, shutdown: CancellationToken) {
        let models_dir = self.models_dir.clone();
        let label_flag_terms = self.label_flag_terms.clone();

        for cfg in self.label_models {
            let db = self.db.clone();
            let fs = self.fs.clone();
            let models_dir = models_dir.clone();
            let flag_terms = label_flag_terms.clone();
            let token = shutdown.clone();
            let cfg_clone = cfg.clone();

            tokio::spawn(async move {
                let labeler = match Self::build_labeler(&cfg_clone, &models_dir) {
                    Some(l) => Arc::new(l),
                    None => return,
                };

                info!("Label worker '{}' started", labeler.name());

                loop {
                    tokio::select! {
                        _ = Self::run_batch(labeler.clone(), &db, &fs, &flag_terms) => {}
                        _ = token.cancelled() => {
                            info!("Label worker '{}' shutting down", labeler.name());
                            return;
                        }
                    }
                }
            });
        }

        tokio::time::sleep(Duration::from_secs(365 * 24 * 60 * 60)).await;
    }

    /// Construct the appropriate [`MediaLabeler`] for a given config entry.
    fn build_labeler(
        cfg: &LabelModelConfig,
        models_dir: &Path,
    ) -> Option<Box<dyn MediaLabeler + Send + Sync>> {
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

    async fn run_batch(
        labeler: Arc<Box<dyn MediaLabeler + Send + Sync>>,
        db: &Database,
        fs: &FileStore,
        label_flag_terms: &[String],
    ) {
        let model_name = labeler.name().to_string();
        let to_label = match db.get_files_missing_labels(&model_name).await {
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
                db.add_labeled_by(&file.id, &model_name)
                    .await
                    .unwrap_or_else(|e| {
                        error!(
                            "Failed to update labeled_by for {}: {}",
                            hex::encode(file.id),
                            e
                        );
                    });
                continue;
            }

            let start = std::time::Instant::now();
            let labeler_clone = labeler.clone();
            let new_labels = match tokio::task::spawn_blocking(move || {
                labeler_clone.label_file(&path, &file.mime_type)
            })
            .await
            {
                Ok(Ok(results)) => {
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
                            FileLabel::new(label, model_name.clone())
                        })
                        .collect::<Vec<_>>()
                }
                Ok(Err(e)) => {
                    let elapsed = start.elapsed();
                    let file_id = file.id.clone();
                    error!(
                        "Label model '{}' failed on {} after {:.2?}: {}",
                        model_name,
                        hex::encode(&file_id),
                        elapsed,
                        e
                    );
                    db.add_labeled_by(&file.id, &model_name)
                        .await
                        .unwrap_or_else(|e| {
                            error!(
                                "Failed to update labeled_by for {}: {}",
                                hex::encode(file.id),
                                e
                            );
                        });
                    continue;
                }
                Err(e) => {
                    let file_id = file.id.clone();
                    error!("Label task for {} panicked: {}", hex::encode(&file_id), e);
                    db.add_labeled_by(&file.id, &model_name)
                        .await
                        .unwrap_or_else(|e| {
                            error!(
                                "Failed to update labeled_by for {}: {}",
                                hex::encode(file.id),
                                e
                            );
                        });
                    continue;
                }
            };

            for label in &new_labels {
                db.add_file_label(&file.id, label)
                    .await
                    .unwrap_or_else(|e| {
                        error!(
                            "Failed to save label '{}' for {}: {}",
                            label.label,
                            hex::encode(&file.id),
                            e
                        );
                    });
            }

            let file_id = file.id.clone();
            db.add_labeled_by(&file_id, &model_name)
                .await
                .unwrap_or_else(|e| {
                    error!(
                        "Failed to update labeled_by for {}: {}",
                        hex::encode(&file_id),
                        e
                    );
                });

            if !label_flag_terms.is_empty() && !new_labels.is_empty() {
                let file_id = file.id.clone();
                let new_state = Database::review_state_for_labels(&new_labels, label_flag_terms);
                if new_state != ReviewState::None {
                    db.set_file_review_state(&file_id, new_state)
                        .await
                        .unwrap_or_else(|e| {
                            error!(
                                "Failed to set review state for {}: {}",
                                hex::encode(&file_id),
                                e
                            );
                        });
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

use super::{BatchResult, next_sleep};
use crate::db::{Database, FileLabel, ReviewState};
use crate::filesystem::FileStore;
use crate::processing::labeling::{GenericLlmLabeler, MediaLabeler, VitLabeler};
use crate::settings::{LabelModelConfig, LabelerType};
use candle_core::Device;
use log::{error, info, warn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

        let mut handles = vec![];

        for cfg in self.label_models {
            let db = self.db.clone();
            let fs = self.fs.clone();
            let models_dir = models_dir.clone();
            let flag_terms = label_flag_terms.clone();
            let token = shutdown.clone();
            let cfg_clone = cfg.clone();

            let handle = tokio::spawn(async move {
                let labeler = match tokio::task::spawn_blocking(move || {
                    Self::build_labeler(&cfg_clone, &models_dir)
                })
                .await
                {
                    Ok(Some(l)) => Arc::new(l),
                    Ok(None) => return,
                    Err(e) => {
                        error!("Label worker panicked during model load: {:?}", e);
                        return;
                    }
                };

                if token.is_cancelled() {
                    return;
                }

                info!("Label worker '{}' started", labeler.name());

                let mut prev_count: usize = 0;
                let mut stall_rounds: u32 = 0;

                loop {
                    let sleep_dur;

                    tokio::select! {
                        batch_result = Self::run_batch(labeler.clone(), &db, &fs, &flag_terms, &token) => {
                            sleep_dur = next_sleep(&batch_result, &mut prev_count, &mut stall_rounds);
                            if let BatchResult::Processed { found, failed } = batch_result
                                && stall_rounds > 0
                            {
                                warn!(
                                    "Label worker '{}': stalled on {} files ({} failed), backing off {:.0?}",
                                    labeler.name(),
                                    found,
                                    failed,
                                    sleep_dur,
                                );
                            }
                        }
                        _ = token.cancelled() => {
                            info!("Label worker '{}' shutting down", labeler.name());
                            return;
                        }
                    }

                    tokio::select! {
                        _ = tokio::time::sleep(sleep_dur) => {}
                        _ = token.cancelled() => {
                            info!("Label worker '{}' shutting down", labeler.name());
                            return;
                        }
                    }
                }
            });

            handles.push(handle);
        }

        for h in handles {
            h.await
                .unwrap_or_else(|e| error!("Label worker task failed: {:?}", e));
        }
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
            LabelerType::GenericLlm {
                api_url,
                model,
                api_key,
                prompt,
                max_image_pixels,
            } => {
                if model.is_empty() {
                    error!("GenericLlm labeler '{}' requires a model name", cfg.name);
                    return None;
                }
                info!("Initializing generic LLM labeler '{}'", cfg.name);
                match GenericLlmLabeler::new(
                    api_url.clone(),
                    model.clone(),
                    api_key.clone(),
                    prompt.clone(),
                    cfg.label_exclude.clone(),
                    cfg.min_confidence,
                    cfg.name.clone(),
                    *max_image_pixels,
                ) {
                    Ok(l) => Some(Box::new(l)),
                    Err(e) => {
                        error!("Failed to initialize LLM labeler '{}': {}", cfg.name, e);
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
        shutdown: &CancellationToken,
    ) -> BatchResult {
        let model_name = labeler.name().to_string();
        let to_label = match db.get_files_missing_labels(&model_name).await {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to query missing labels for '{}': {}", model_name, e);
                return BatchResult::Idle;
            }
        };

        if to_label.is_empty() {
            return BatchResult::Idle;
        }

        let found = to_label.len();
        let mut failed = 0;
        info!("{} files missing labels for model '{}'", found, model_name);

        for file in to_label {
            if shutdown.is_cancelled() {
                return BatchResult::Processed { found, failed };
            }
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
                failed += 1;
                continue;
            }

            info!(
                "Labeling file={} model={} mime={} path={}",
                hex::encode(&file.id),
                model_name,
                file.mime_type,
                path.display(),
            );
            let start = std::time::Instant::now();
            let labeler_clone = labeler.clone();
            let new_labels = match tokio::time::timeout(
                std::time::Duration::from_secs(300),
                tokio::task::spawn_blocking(move || {
                    labeler_clone.label_file(&path, &file.mime_type)
                }),
            )
            .await
            {
                Ok(Ok(Ok(results))) => {
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
                Ok(Ok(Err(e))) => {
                    let elapsed = start.elapsed();
                    let file_id = file.id.clone();
                    let is_transient = e.is_transient();
                    if e.is_skip() {
                        info!(
                            "Label model '{}' skipped {}: {}",
                            model_name,
                            hex::encode(&file_id),
                            e,
                        );
                    } else {
                        error!(
                            "Label model '{}' failed on {} after {:.2?}: {} [{}]",
                            model_name,
                            hex::encode(&file_id),
                            elapsed,
                            e,
                            if is_transient {
                                "transient"
                            } else {
                                "permanent"
                            },
                        );
                    }
                    if !is_transient {
                        db.add_labeled_by(&file_id, &model_name)
                            .await
                            .unwrap_or_else(|e| {
                                error!(
                                    "Failed to mark failed file {} as labeled: {}",
                                    hex::encode(&file_id),
                                    e
                                );
                            });
                    }
                    failed += 1;
                    continue;
                }
                Ok(Err(e)) => {
                    let file_id = file.id.clone();
                    error!("Label task for {} panicked: {}", hex::encode(&file_id), e);
                    // Panics are likely caused by corrupt media (e.g. ffmpeg
                    // segfault), so always mark as done to prevent retry loops.
                    db.add_labeled_by(&file_id, &model_name)
                        .await
                        .unwrap_or_else(|e| {
                            error!(
                                "Failed to mark panicked file {} as labeled: {}",
                                hex::encode(&file_id),
                                e
                            );
                        });
                    failed += 1;
                    continue;
                }
                Err(_) => {
                    let file_id = file.id.clone();
                    error!(
                        "Label model '{}' timed out on {} after {:.2?}",
                        model_name,
                        hex::encode(&file_id),
                        start.elapsed()
                    );
                    // Timeouts are treated as transient — the file may succeed
                    // on a less busy system. It will be retried with backoff.
                    failed += 1;
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

            if !label_flag_terms.is_empty()
                && !new_labels.is_empty()
                && Database::review_state_for_labels(&new_labels, label_flag_terms)
                    == ReviewState::LabelFlagged
            {
                db.flag_file_for_review(&file.id).await.unwrap_or_else(|e| {
                    error!(
                        "Failed to set review state for {}: {}",
                        hex::encode(&file.id),
                        e
                    );
                });
            }
        }

        BatchResult::Processed { found, failed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::FileUpload;
    use crate::processing::labeling::LabelError;
    use crate::settings::Settings;
    use sha2::{Digest, Sha256};
    use sqlx::Executor;
    use std::collections::HashMap;
    use tokio::sync::RwLock;

    struct FixedLabeler(&'static str);

    impl MediaLabeler for FixedLabeler {
        fn name(&self) -> &str {
            "nsfw"
        }

        fn label_exclude(&self) -> &[String] {
            &[]
        }

        fn min_confidence(&self) -> f32 {
            0.4
        }

        fn label_file(
            &self,
            _path: &Path,
            _mime_type: &str,
        ) -> Result<HashMap<String, f32>, LabelError> {
            Ok(HashMap::from([(self.0.to_string(), 0.9)]))
        }
    }

    async fn test_db() -> Database {
        let url = std::env::var("ROUTE96_TEST_DATABASE")
            .expect("set ROUTE96_TEST_DATABASE=mysql://root:root@127.0.0.1:3399");
        let name = format!("r96_label_{}", uuid::Uuid::new_v4().simple());
        let admin = Database::new(&url).await.unwrap();
        admin
            .pool
            .execute(format!("create database {}", name).as_str())
            .await
            .unwrap();
        let db = Database::new(&format!("{}/{}", url, name)).await.unwrap();
        db.migrate().await.unwrap();
        db
    }

    async fn add(db: &Database, fs: &FileStore, content: &[u8], state: ReviewState) -> Vec<u8> {
        let id = Sha256::digest(content).to_vec();
        let path = fs.get(&id);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, content).await.unwrap();
        let upload = FileUpload {
            id: id.clone(),
            size: content.len() as u64,
            mime_type: "image/png".to_string(),
            review_state: state,
            created: chrono::Utc::now(),
            ..Default::default()
        };
        db.add_file(&upload, None).await.unwrap();
        id
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn relabel_does_not_reflag_reviewed_files() {
        let dir = tempfile::tempdir().unwrap();
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "storage_dir": dir.path(),
            "database": "",
            "max_upload_bytes": 1024,
            "public_url": "",
        }))
        .unwrap();
        let fs = FileStore::new(Arc::new(RwLock::new(settings)));
        let db = test_db().await;

        let reviewed = add(&db, &fs, b"reviewed", ReviewState::Reviewed).await;
        let fresh = add(&db, &fs, b"fresh", ReviewState::None).await;

        let labeler: Arc<Box<dyn MediaLabeler + Send + Sync>> =
            Arc::new(Box::new(FixedLabeler("nsfw")));
        let terms = vec!["nsfw".to_string()];
        let result =
            LabelFiles::run_batch(labeler, &db, &fs, &terms, &CancellationToken::new()).await;
        assert!(matches!(
            result,
            BatchResult::Processed {
                found: 2,
                failed: 0
            }
        ));

        let state = |id: Vec<u8>| {
            let db = db.clone();
            async move { db.get_file(&id).await.unwrap().unwrap().review_state }
        };
        assert_eq!(state(reviewed).await, ReviewState::Reviewed);
        assert_eq!(state(fresh).await, ReviewState::LabelFlagged);
    }
}

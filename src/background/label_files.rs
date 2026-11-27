use super::{BatchResult, next_sleep};
use crate::db::{Database, FileLabel, LabelDeletion, LabelModelScope, ReviewState};
use crate::filesystem::FileStore;
use crate::processing::labeling::{
    GenericLlmLabeler, MediaLabeler, TaxonomyLabeler, TaxonomyModel, VitLabeler,
};
use crate::settings::{LabelModelConfig, LabelerType, Settings};
use candle_core::Device;
use log::{error, info, warn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

type FileLabels = (Vec<u8>, Vec<(String, f32)>);

const DEFAULT_DELETE_MIN_CONFIDENCE: f32 = 0.8;

#[derive(Clone, Debug, Default)]
pub struct DeletePolicy {
    pub terms: Vec<String>,
    pub min_confidence: f32,
    pub dry_run: bool,
}

impl DeletePolicy {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            terms: s.label_delete_terms.clone().unwrap_or_default(),
            min_confidence: s
                .label_delete_min_confidence
                .unwrap_or(DEFAULT_DELETE_MIN_CONFIDENCE),
            dry_run: s.label_delete_dry_run.unwrap_or(true),
        }
    }

    fn for_model(&self, model: &str, moderates: bool) -> Self {
        Self {
            terms: terms_for_model(&self.terms, model, moderates),
            ..self.clone()
        }
    }
}

pub fn terms_for_model(terms: &[String], model: &str, moderates: bool) -> Vec<String> {
    let model = model.to_lowercase();
    terms
        .iter()
        .filter_map(|t| {
            let trimmed = t.trim();
            match trimmed.split_once(':') {
                Some((scope, rest)) if scope.trim().to_lowercase() == model => {
                    Some(rest.trim().to_string())
                }
                Some(_) => None,
                None if moderates => Some(trimmed.to_string()),
                None => None,
            }
        })
        .filter(|t| !t.is_empty())
        .collect()
}

pub const TAXONOMY_MODEL_NAME: &str = "taxonomy-v1";

pub fn purge_scope(term: &str, models: &[LabelModelConfig]) -> (String, LabelModelScope) {
    let term = term.trim();
    let lower = term.to_lowercase();
    let scoped = models.iter().map(|m| m.name.as_str()).find_map(|name| {
        lower
            .strip_prefix(&format!("{}:", name.to_lowercase()))
            .map(|rest| (rest.trim().to_string(), LabelModelScope::Only(name.to_string())))
    });
    if let Some(found) = scoped {
        return found;
    }
    let mut skip: Vec<String> = models
        .iter()
        .filter(|m| !m.moderates())
        .map(|m| m.name.clone())
        .collect();
    if !skip.iter().any(|m| m == TAXONOMY_MODEL_NAME) {
        skip.push(TAXONOMY_MODEL_NAME.to_string());
    }
    (term.to_string(), LabelModelScope::Except(skip))
}

pub(crate) struct DeleteMatch {
    pub(crate) term: String,
    pub(crate) label: String,
    pub(crate) confidence: f32,
    pub(crate) reason: Option<String>,
}

pub struct LabelFiles {
    db: Database,
    fs: FileStore,
    models_dir: PathBuf,
    label_models: Vec<LabelModelConfig>,
    settings: Arc<RwLock<Settings>>,
}

impl LabelFiles {
    pub fn new(
        db: Database,
        fs: FileStore,
        models_dir: PathBuf,
        label_models: Vec<LabelModelConfig>,
        settings: Arc<RwLock<Settings>>,
    ) -> Self {
        Self {
            db,
            fs,
            models_dir,
            label_models,
            settings,
        }
    }

    /// Spawn a dedicated tokio task **per model** so they can process batches
    /// in parallel. Each task owns its labeler (and any CUDA state it holds).
    pub async fn process(self, shutdown: CancellationToken) {
        let models_dir = self.models_dir.clone();
        let settings = self.settings.clone();

        let mut handles = vec![];

        for cfg in self.label_models {
            let db = self.db.clone();
            let fs = self.fs.clone();
            let models_dir = models_dir.clone();
            let settings = settings.clone();
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
                        batch_result = async {
                            let (flag_terms, delete) = {
                                let s = settings.read().await;
                                (s.label_flag_terms.clone().unwrap_or_default(), DeletePolicy::from_settings(&s))
                            };
                            Self::run_batch(labeler.clone(), &db, &fs, &flag_terms, &delete, &token).await
                        } => {
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
            LabelerType::Taxonomy {
                head_path,
                hf_repo,
                backbone,
            } => {
                info!("Loading taxonomy label model '{}'", cfg.name);
                let device = Device::cuda_if_available(0).unwrap_or_else(|e| {
                    error!("Failed to initialize CUDA device: {}", e);
                    Device::Cpu
                });
                match TaxonomyModel::resolve(
                    models_dir,
                    head_path.as_deref(),
                    hf_repo.as_deref(),
                    backbone.as_deref(),
                    device,
                ) {
                    Ok(model) => Some(Box::new(TaxonomyLabeler::new(
                        model,
                        cfg.name.clone(),
                        cfg.label_exclude.clone(),
                        cfg.min_confidence,
                    ))),
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

    pub(crate) async fn run_batch(
        labeler: Arc<Box<dyn MediaLabeler + Send + Sync>>,
        db: &Database,
        fs: &FileStore,
        label_flag_terms: &[String],
        delete: &DeletePolicy,
        shutdown: &CancellationToken,
    ) -> BatchResult {
        let model_name = labeler.name().to_string();
        let flag_terms = terms_for_model(label_flag_terms, &model_name, labeler.moderates());
        let label_flag_terms = flag_terms.as_slice();
        let delete = &delete.for_model(&model_name, labeler.moderates());
        if !delete.dry_run {
            Self::apply_pending(db, fs, &model_name, delete).await;
        }
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
                            (FileLabel::scored(label, model_name.clone(), score), score)
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

            for (label, _) in &new_labels {
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

            let labels_only: Vec<FileLabel> = new_labels.iter().map(|(l, _)| l.clone()).collect();
            let delete_hit = Self::delete_match(&new_labels, delete);
            let flagged = !labels_only.is_empty()
                && (Database::review_state_for_labels(&labels_only, label_flag_terms)
                    == ReviewState::LabelFlagged
                    || Self::delete_match(
                        &new_labels,
                        &DeletePolicy {
                            min_confidence: 0.0,
                            ..delete.clone()
                        },
                    )
                    .is_some());
            if flagged {
                db.flag_file_for_review(&file.id).await.unwrap_or_else(|e| {
                    error!(
                        "Failed to set review state for {}: {}",
                        hex::encode(&file.id),
                        e
                    );
                });
            }

            if let Some(hit) = delete_hit {
                Self::auto_delete(db, fs, &file.id, &model_name, &hit, delete.dry_run).await;
            }
        }

        BatchResult::Processed { found, failed }
    }

    async fn apply_pending(db: &Database, fs: &FileStore, model: &str, delete: &DeletePolicy) {
        let pending = match db.pending_dry_run_deletions(model).await {
            Ok(v) => v,
            Err(e) => {
                error!("Label delete: failed to read pending matches: {}", e);
                return;
            }
        };
        let mut by_file: Vec<FileLabels> = Vec::new();
        for row in pending {
            match by_file.iter_mut().find(|(f, _)| *f == row.file) {
                Some((_, labels)) => labels.push((row.label, row.confidence)),
                None => by_file.push((row.file, vec![(row.label, row.confidence)])),
            }
        }
        for (file, labels) in by_file {
            let Some((term, label, confidence)) =
                Database::combination_match(&labels, &delete.terms, delete.min_confidence)
            else {
                continue;
            };
            let hit = DeleteMatch {
                term: term.clone(),
                label,
                confidence,
                reason: None,
            };
            Self::auto_delete(db, fs, &file, model, &hit, false).await;
        }
    }

    fn delete_match(labels: &[(FileLabel, f32)], delete: &DeletePolicy) -> Option<DeleteMatch> {
        let scored: Vec<(String, f32)> = labels.iter().map(|(l, s)| (l.label.clone(), *s)).collect();
        Database::combination_match(&scored, &delete.terms, delete.min_confidence).map(
            |(term, label, confidence)| DeleteMatch {
                term: term.clone(),
                label,
                confidence,
                reason: None,
            },
        )
    }

    pub(crate) async fn auto_delete(
        db: &Database,
        fs: &FileStore,
        id: &Vec<u8>,
        model: &str,
        hit: &DeleteMatch,
        dry_run: bool,
    ) {
        let hash = hex::encode(id);
        let pubkeys = match db.get_file_owners(id).await {
            Ok(owners) => owners
                .iter()
                .map(|u| hex::encode(&u.pubkey))
                .collect::<Vec<_>>()
                .join(","),
            Err(e) => {
                error!("Label delete: failed to read owners of {}: {}", hash, e);
                return;
            }
        };
        let reason = hit
            .reason
            .clone()
            .unwrap_or_else(|| format!("label:{} ({} {:.2})", hit.term, model, hit.confidence));
        let record = LabelDeletion {
            file: id.clone(),
            pubkeys: pubkeys.clone(),
            term: hit.term.clone(),
            label: hit.label.clone(),
            model: model.to_string(),
            confidence: hit.confidence,
            reason: reason.clone(),
            dry_run,
            created: None,
        };
        if let Err(e) = db.record_label_deletion(&record).await {
            error!(
                "Label delete: failed to write audit row for {}: {}",
                hash, e
            );
            return;
        }
        if dry_run {
            info!(
                "Label delete (dry run): would delete file={} owners=[{}] label={} model={} confidence={:.2} reason={}",
                hash, pubkeys, hit.label, model, hit.confidence, reason
            );
            return;
        }
        warn!(
            "Label delete: deleting file={} owners=[{}] label={} model={} confidence={:.2} reason={}",
            hash, pubkeys, hit.label, model, hit.confidence, reason
        );
        if let Err(e) = db.ban_file(id).await {
            error!("Label delete: failed to ban {}: {}", hash, e);
            return;
        }
        if let Err(e) = fs.delete(id).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            error!("Label delete: failed to remove {} from disk: {}", hash, e);
        }
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

    struct ScoredLabeler(&'static str, f32);

    impl MediaLabeler for ScoredLabeler {
        fn name(&self) -> &str {
            "nsfw"
        }

        fn label_exclude(&self) -> &[String] {
            &[]
        }

        fn min_confidence(&self) -> f32 {
            0.1
        }

        fn label_file(
            &self,
            _path: &Path,
            _mime_type: &str,
        ) -> Result<HashMap<String, f32>, LabelError> {
            Ok(HashMap::from([(self.0.to_string(), self.1)]))
        }
    }

    struct TaxonomyFixed;

    impl MediaLabeler for TaxonomyFixed {
        fn name(&self) -> &str {
            TAXONOMY_MODEL_NAME
        }

        fn label_exclude(&self) -> &[String] {
            &[]
        }

        fn min_confidence(&self) -> f32 {
            0.0
        }

        fn label_file(
            &self,
            _path: &Path,
            _mime_type: &str,
        ) -> Result<HashMap<String, f32>, LabelError> {
            Ok(HashMap::from([
                ("nude".to_string(), 0.95),
                ("breast".to_string(), 0.9),
            ]))
        }

        fn moderates(&self) -> bool {
            false
        }
    }

    async fn run_taxonomy(db: &Database, fs: &FileStore, terms: &[&str]) {
        let terms: Vec<String> = terms.iter().map(|t| t.to_string()).collect();
        let labeler: Arc<Box<dyn MediaLabeler + Send + Sync>> = Arc::new(Box::new(TaxonomyFixed));
        let delete = DeletePolicy {
            terms: terms.clone(),
            min_confidence: 0.8,
            dry_run: false,
        };
        LabelFiles::run_batch(labeler, db, fs, &terms, &delete, &CancellationToken::new()).await;
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn taxonomy_nude_neither_flags_nor_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let id = add_owned(&db, &fs, b"taxonomy", ReviewState::None).await;

        run_taxonomy(&db, &fs, &["nude", "breast", "nsfw"]).await;

        assert!(fs.get(&id).exists());
        let f = db.get_file(&id).await.unwrap().unwrap();
        assert!(!f.banned);
        assert_eq!(f.review_state, ReviewState::None);
        assert!(db.list_label_deletions(None, 0, 10).await.unwrap().is_empty());
        let mut labels = db.get_file_labels(&id).await.unwrap();
        labels.sort_by(|a, b| a.label.cmp(&b.label));
        let stored: Vec<(String, String, Option<f32>)> = labels
            .into_iter()
            .map(|l| (l.label, l.model, l.confidence))
            .collect();
        assert_eq!(
            stored,
            vec![
                ("breast".to_string(), TAXONOMY_MODEL_NAME.to_string(), Some(0.9)),
                ("nude".to_string(), TAXONOMY_MODEL_NAME.to_string(), Some(0.95)),
            ]
        );

        let models: Vec<LabelModelConfig> = serde_json::from_value(serde_json::json!([
            {"name": "nsfw", "type": "vit", "hf_repo": "v0l/route96-nsfw-vit224"},
            {"name": TAXONOMY_MODEL_NAME, "type": "taxonomy", "head_path": "/x"}
        ]))
        .unwrap();
        let (term, scope) = purge_scope("nude", &models);
        assert!(db.files_with_label_matching(&term, &scope).await.unwrap().is_empty());
        let (term, scope) = purge_scope("taxonomy-v1:nude", &models);
        assert_eq!(db.files_with_label_matching(&term, &scope).await.unwrap().len(), 1);
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn taxonomy_term_set_on_purpose_flags_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let id = add_owned(&db, &fs, b"scoped", ReviewState::None).await;

        run_taxonomy(&db, &fs, &["Taxonomy-v1:nude"]).await;

        assert!(!fs.get(&id).exists());
        assert!(db.is_file_banned(&id).await.unwrap());
        let rows = db.list_label_deletions(Some(false), 0, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model, TAXONOMY_MODEL_NAME);
        assert_eq!(rows[0].term, "nude");
    }

    #[test]
    fn scoped_terms_reach_only_their_model() {
        let terms: Vec<String> = ["gore", "taxonomy-v1: nude", "NSFW:porn", "taxonomy-v1:"]
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(terms_for_model(&terms, "taxonomy-v1", false), ["nude"]);
        assert_eq!(terms_for_model(&terms, "nsfw", true), ["gore", "porn"]);
        assert_eq!(terms_for_model(&terms, "vit224", true), ["gore"]);
    }

    #[test]
    fn purge_scope_skips_taxonomy_models_unless_named() {
        let models: Vec<LabelModelConfig> = serde_json::from_value(serde_json::json!([
            {"name": "nsfw", "type": "vit", "hf_repo": "v0l/route96-nsfw-vit224"},
            {"name": "tax2", "type": "taxonomy", "hf_repo": "v0l/route96-taxonomy"}
        ]))
        .unwrap();
        assert!(models[0].moderates());
        assert!(!models[1].moderates());
        assert_eq!(
            purge_scope(" gore ", &models),
            (
                "gore".to_string(),
                LabelModelScope::Except(vec!["tax2".to_string(), TAXONOMY_MODEL_NAME.to_string()])
            )
        );
        assert_eq!(
            purge_scope("TAX2:nude", &models),
            ("nude".to_string(), LabelModelScope::Only("tax2".to_string()))
        );
    }

    fn policy(dry_run: bool) -> DeletePolicy {
        DeletePolicy {
            terms: vec!["gore".to_string()],
            min_confidence: 0.8,
            dry_run,
        }
    }

    fn test_store(dir: &Path) -> FileStore {
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "storage_dir": dir,
            "database": "",
            "max_upload_bytes": 1024,
            "public_url": "",
        }))
        .unwrap();
        FileStore::new(Arc::new(RwLock::new(settings)))
    }

    async fn run(db: &Database, fs: &FileStore, score: f32, policy: &DeletePolicy) {
        let labeler: Arc<Box<dyn MediaLabeler + Send + Sync>> =
            Arc::new(Box::new(ScoredLabeler("graphic gore", score)));
        LabelFiles::run_batch(labeler, db, fs, &[], policy, &CancellationToken::new()).await;
    }

    async fn add_owned(db: &Database, fs: &FileStore, content: &[u8], state: ReviewState) -> Vec<u8> {
        let id = add(db, fs, content, state).await;
        let user = db.upsert_user(&vec![7u8; 32]).await.unwrap();
        sqlx::query("insert into user_uploads(file, user_id) values(?, ?)")
            .bind(&id)
            .bind(user)
            .execute(&db.pool)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn delete_term_match_removes_file_and_writes_audit() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let id = add_owned(&db, &fs, b"bad", ReviewState::None).await;

        run(&db, &fs, 0.95, &policy(false)).await;

        assert!(!fs.get(&id).exists());
        let row = db.get_file(&id).await.unwrap();
        assert!(row.map(|f| f.banned).unwrap_or(true));
        let audit = db.list_label_deletions(Some(false), 0, 10).await.unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].file, id);
        assert_eq!(audit[0].reason, "label:gore (nsfw 0.95)");
        assert_eq!(audit[0].pubkeys, hex::encode(vec![7u8; 32]));
        assert!(db.get_file_owners(&id).await.unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn deleted_hash_cannot_be_reuploaded() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let id = add_owned(&db, &fs, b"bad", ReviewState::None).await;

        run(&db, &fs, 0.95, &policy(false)).await;

        assert!(db.is_file_banned(&id).await.unwrap());
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn delete_match_below_confidence_only_flags() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let id = add_owned(&db, &fs, b"weak", ReviewState::None).await;

        run(&db, &fs, 0.5, &policy(false)).await;

        assert!(fs.get(&id).exists());
        let f = db.get_file(&id).await.unwrap().unwrap();
        assert!(!f.banned);
        assert_eq!(f.review_state, ReviewState::LabelFlagged);
        assert!(db.list_label_deletions(None, 0, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn reviewed_file_is_auto_deleted_too() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let id = add_owned(&db, &fs, b"ok", ReviewState::Reviewed).await;

        run(&db, &fs, 0.99, &policy(false)).await;

        assert!(!fs.get(&id).exists());
        assert!(db.is_file_banned(&id).await.unwrap());
        let rows = db.list_label_deletions(Some(false), 0, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].file, id);
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn dry_run_keeps_file_flags_and_records_would_delete() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let id = add_owned(&db, &fs, b"dry", ReviewState::None).await;

        run(&db, &fs, 0.95, &policy(true)).await;

        assert!(fs.get(&id).exists());
        let f = db.get_file(&id).await.unwrap().unwrap();
        assert!(!f.banned);
        assert_eq!(f.review_state, ReviewState::LabelFlagged);
        assert_eq!(db.get_file_owners(&id).await.unwrap().len(), 1);
        let would = db.list_label_deletions(Some(true), 0, 10).await.unwrap();
        assert_eq!(would.len(), 1);
        assert_eq!(would[0].file, id);
        assert!(db.list_label_deletions(Some(false), 0, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn turning_dry_run_off_deletes_pending_matches() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let id = add_owned(&db, &fs, b"pending", ReviewState::None).await;
        let reviewed = add_owned(&db, &fs, b"pending reviewed", ReviewState::None).await;

        run(&db, &fs, 0.95, &policy(true)).await;
        assert!(fs.get(&id).exists());
        db.set_file_review_state(&reviewed, ReviewState::Reviewed)
            .await
            .unwrap();

        run(&db, &fs, 0.95, &policy(false)).await;

        assert!(!fs.get(&id).exists());
        assert!(db.is_file_banned(&id).await.unwrap());
        let real = db.list_label_deletions(Some(false), 0, 10).await.unwrap();
        assert_eq!(real.len(), 2);
        assert!(!fs.get(&reviewed).exists());
        assert!(db.is_file_banned(&reviewed).await.unwrap());
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn purge_deletes_every_existing_file_with_matching_label() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let a = add_owned(&db, &fs, b"a", ReviewState::None).await;
        let b = add_owned(&db, &fs, b"b", ReviewState::None).await;
        let keep = add_owned(&db, &fs, b"keep", ReviewState::None).await;
        let reviewed = add_owned(&db, &fs, b"rev", ReviewState::Reviewed).await;
        for (id, label) in [(&a, "Graphic_Gore"), (&b, "gore"), (&keep, "cat"), (&reviewed, "gore")] {
            db.add_file_label(id, &FileLabel::new(label.to_string(), "nsfw".to_string()))
                .await
                .unwrap();
        }

        let files = db
            .files_with_label_matching("gore", &LabelModelScope::Except(vec![]))
            .await
            .unwrap();
        assert_eq!(files.len(), 3);
        for (id, label, model) in files {
            let hit = DeleteMatch {
                term: "gore".to_string(),
                label,
                confidence: 1.0,
                reason: Some("label:gore (admin purge)".to_string()),
            };
            LabelFiles::auto_delete(&db, &fs, &id, &model, &hit, false).await;
        }

        for id in [&a, &b, &reviewed] {
            assert!(!fs.get(id).exists());
            assert!(db.is_file_banned(id).await.unwrap());
        }
        assert!(fs.get(&keep).exists());
        let rows = db.list_label_deletions(Some(false), 0, 10).await.unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.reason == "label:gore (admin purge)"));
        assert!(
            db.files_with_label_matching("100%", &LabelModelScope::Except(vec![]))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn combination_term_needs_every_part_above_threshold() {
        let terms = vec!["beach+dog".to_string()];
        let both = vec![("Beach".to_string(), 0.9), ("Dog".to_string(), 0.85)];
        let (term, label, confidence) = Database::combination_match(&both, &terms, 0.8).unwrap();
        assert_eq!(term, "beach+dog");
        assert_eq!(label, "Beach, Dog");
        assert_eq!(confidence, 0.85);
        let weak = vec![("Beach".to_string(), 0.9), ("Dog".to_string(), 0.5)];
        assert!(Database::combination_match(&weak, &terms, 0.8).is_none());
        let one = vec![("Beach".to_string(), 0.9)];
        assert!(Database::combination_match(&one, &terms, 0.8).is_none());
        let plain = vec!["gore".to_string()];
        assert!(Database::combination_match(&[("Gore".to_string(), 0.9)], &plain, 0.8).is_some());
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
        let result = LabelFiles::run_batch(
            labeler,
            &db,
            &fs,
            &terms,
            &DeletePolicy::default(),
            &CancellationToken::new(),
        )
        .await;
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

use super::{BatchResult, next_sleep};
use crate::db::{Database, FileLabel, LabelDeletion, ReviewState};
use crate::filesystem::FileStore;
use crate::processing::labeling::{GenericLlmLabeler, MediaLabeler, VitLabeler};
use crate::settings::{LabelModelConfig, LabelerType, Settings};
use candle_core::Device;
use log::{error, info, warn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

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
                            (FileLabel::new(label, model_name.clone()), score)
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
                    || new_labels
                        .iter()
                        .any(|(l, _)| Database::matching_term(&l.label, &delete.terms).is_some()));
            if flagged {
                db.flag_file_for_review(&file.id).await.unwrap_or_else(|e| {
                    error!(
                        "Failed to set review state for {}: {}",
                        hex::encode(&file.id),
                        e
                    );
                });
            }

            if let Some(hit) = delete_hit
                && file.review_state != ReviewState::Reviewed
            {
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
        let mut seen = std::collections::HashSet::new();
        for row in pending {
            if !seen.insert(row.file.clone()) || row.confidence < delete.min_confidence {
                continue;
            }
            let Some(term) = Database::matching_term(&row.label, &delete.terms) else {
                continue;
            };
            let hit = DeleteMatch {
                term: term.clone(),
                label: row.label.clone(),
                confidence: row.confidence,
                reason: None,
            };
            Self::auto_delete(db, fs, &row.file, model, &hit, false).await;
        }
    }

    fn delete_match(labels: &[(FileLabel, f32)], delete: &DeletePolicy) -> Option<DeleteMatch> {
        labels
            .iter()
            .filter(|(_, score)| *score >= delete.min_confidence)
            .filter_map(|(l, score)| {
                Database::matching_term(&l.label, &delete.terms).map(|t| DeleteMatch {
                    term: t.clone(),
                    label: l.label.clone(),
                    confidence: *score,
                    reason: None,
                })
            })
            .max_by(|a, b| a.confidence.total_cmp(&b.confidence))
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
    async fn reviewed_file_is_never_auto_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let fs = test_store(dir.path());
        let db = test_db().await;
        let id = add_owned(&db, &fs, b"ok", ReviewState::Reviewed).await;
        let fresh = add_owned(&db, &fs, b"not reviewed", ReviewState::None).await;

        run(&db, &fs, 0.99, &policy(false)).await;

        assert!(!fs.get(&fresh).exists());
        assert!(db.is_file_banned(&fresh).await.unwrap());
        assert!(fs.get(&id).exists());
        let f = db.get_file(&id).await.unwrap().unwrap();
        assert!(!f.banned);
        assert_eq!(f.review_state, ReviewState::Reviewed);
        let rows = db.list_label_deletions(None, 0, 10).await.unwrap();
        assert!(rows.iter().all(|r| r.file != id));
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
        assert_eq!(real.len(), 1);
        assert_eq!(real[0].file, id);
        assert!(fs.get(&reviewed).exists());
        assert!(!db.is_file_banned(&reviewed).await.unwrap());
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

        let files = db.files_with_label_matching("gore").await.unwrap();
        assert_eq!(files.len(), 2);
        for (id, label, model) in files {
            let hit = DeleteMatch {
                term: "gore".to_string(),
                label,
                confidence: 1.0,
                reason: Some("label:gore (admin purge)".to_string()),
            };
            LabelFiles::auto_delete(&db, &fs, &id, &model, &hit, false).await;
        }

        for id in [&a, &b] {
            assert!(!fs.get(id).exists());
            assert!(db.is_file_banned(id).await.unwrap());
        }
        assert!(fs.get(&keep).exists());
        assert!(fs.get(&reviewed).exists());
        let rows = db.list_label_deletions(Some(false), 0, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.reason == "label:gore (admin purge)"));
        assert!(db.files_with_label_matching("100%").await.unwrap().is_empty());
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

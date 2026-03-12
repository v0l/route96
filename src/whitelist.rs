use crate::db::Database;
use crate::settings::WhitelistMode;
use log::{error, info, warn};
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};
use tokio_util::sync::CancellationToken;

/// Runtime whitelist derived from [`WhitelistMode`].
///
/// Cloning is cheap — all state lives behind `Arc`.
#[derive(Clone)]
pub struct Whitelist {
    inner: Arc<WhitelistInner>,
}

enum WhitelistInner {
    /// Server is open; every pubkey is allowed.
    Open,
    /// Static in-memory set loaded from config or a hot-reloaded file.
    InMemory(RwLock<HashSet<String>>),
    /// Entries live in the database, managed via the admin UI.
    Database(Database),
}

// Manual Default so callers can construct an open whitelist with `Whitelist::default()`.
impl Default for Whitelist {
    fn default() -> Self {
        Self {
            inner: Arc::new(WhitelistInner::Open),
        }
    }
}

impl Whitelist {
    /// Build a [`Whitelist`] from the configured [`WhitelistMode`].
    ///
    /// - `None` → open server (no restrictions).
    /// - `Database` → DB-backed; `db` must be `Some`.
    /// - `Static(pubkeys)` → in-memory set; `db` is not used.
    /// - `File(path)` → starts empty; call [`start_file_watcher`] to load and
    ///   hot-reload the file. `db` is not used.
    pub fn from_mode(mode: Option<&WhitelistMode>, db: Option<&Database>) -> Self {
        match mode {
            None => Self::default(),
            Some(WhitelistMode::Database) => Self {
                inner: Arc::new(WhitelistInner::Database(
                    db.expect("Database whitelist mode requires a database connection").clone(),
                )),
            },
            Some(WhitelistMode::Static(pubkeys)) => {
                let set: HashSet<String> = pubkeys.iter().cloned().collect();
                Self {
                    inner: Arc::new(WhitelistInner::InMemory(RwLock::new(set))),
                }
            }
            Some(WhitelistMode::File(_)) => {
                // Starts empty; the file watcher will populate it.
                Self {
                    inner: Arc::new(WhitelistInner::InMemory(RwLock::new(HashSet::new()))),
                }
            }
        }
    }

    /// Returns `true` if `pubkey_hex` is permitted to upload.
    pub async fn is_allowed(&self, pubkey_hex: &str) -> bool {
        match self.inner.as_ref() {
            WhitelistInner::Open => true,
            WhitelistInner::InMemory(set) => set.read().await.contains(pubkey_hex),
            WhitelistInner::Database(db) => match db.whitelist_contains(pubkey_hex).await {
                Ok(found) => found,
                Err(e) => {
                    warn!("DB whitelist check failed, failing open: {}", e);
                    true
                }
            },
        }
    }

    async fn replace_all(&self, new_set: HashSet<String>) {
        if let WhitelistInner::InMemory(lock) = self.inner.as_ref() {
            *lock.write().await = new_set;
        }
    }

    /// Spawn a background task that loads `path` immediately and then watches
    /// it for changes, hot-reloading on every write.
    ///
    /// Should only be called when the mode is [`WhitelistMode::File`].
    pub async fn watch_file(self, path: PathBuf, shutdown: CancellationToken) {
        let this = self;
        async move {
            let mut last_modified: Option<SystemTime> = None;
            info!("Starting whitelist watcher for {}", path.display());

            // Initial load
            if let Ok(md) = tokio::fs::metadata(&path).await
                && let Ok(modified) = md.modified()
                && let Ok(contents) = tokio::fs::read_to_string(&path).await
            {
                this.replace_all(parse_whitelist_file(&contents)).await;
                last_modified = Some(modified);
                info!("Loaded whitelist from {}", path.display());
            }

            // Event-driven watching; fall back to polling if it fails.
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let mut watcher = match RecommendedWatcher::new(
                move |res| {
                    if let Err(e) = tx.send(res) {
                        error!("Whitelist watcher channel error: {}", e);
                    }
                },
                NotifyConfig::default(),
            ) {
                Ok(w) => w,
                Err(e) => {
                    warn!("Falling back to polling: failed to create file watcher: {}", e);
                    return fallback_polling(this, path, last_modified, shutdown).await;
                }
            };

            let target_name = match path.file_name().map(|s| s.to_os_string()) {
                Some(n) => n,
                None => {
                    warn!("Invalid whitelist file path: {}", path.display());
                    return fallback_polling(this, path, last_modified, shutdown).await;
                }
            };
            let watch_path = path.parent().map(|p| p.to_path_buf()).unwrap_or(path.clone());

            if let Err(e) = watcher.watch(&watch_path, RecursiveMode::NonRecursive) {
                warn!("Falling back to polling: failed to watch {}: {}", path.display(), e);
                return fallback_polling(this, path, last_modified, shutdown).await;
            }

            let mut pending_change = false;
            let mut last_evt: Option<Instant> = None;
            let mut debounce = tokio::time::interval(Duration::from_millis(300));
            debounce.tick().await; // first tick fires immediately; skip it

            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        info!("Stopping whitelist watcher");
                        break;
                    }
                    Some(evt) = rx.recv() => {
                        match evt {
                            Ok(event) => {
                                let relevant = event.paths.iter().any(|p| {
                                    p.file_name().map(|n| n == target_name.as_os_str()).unwrap_or(false)
                                });
                                if !relevant { continue; }
                                match event.kind {
                                    EventKind::Modify(_)
                                    | EventKind::Create(_)
                                    | EventKind::Remove(_)
                                    | EventKind::Access(notify::event::AccessKind::Close(
                                        notify::event::AccessMode::Write,
                                    )) => {
                                        pending_change = true;
                                        last_evt = Some(Instant::now());
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => warn!("Watcher error: {}", e),
                        }
                    }
                    _ = debounce.tick() => {
                        if pending_change {
                            if let Some(t) = last_evt && t.elapsed() < Duration::from_millis(250) {
                                continue;
                            }
                            match tokio::fs::read_to_string(&path).await {
                                Ok(contents) => {
                                    this.replace_all(parse_whitelist_file(&contents)).await;
                                    info!("Reloaded whitelist from {} (debounced)", path.display());
                                }
                                Err(e) => {
                                    warn!("Failed to read whitelist file {}: {}", path.display(), e);
                                }
                            }
                            pending_change = false;
                        }
                    }
                }
            }
        }
        .await
    }
}

pub(crate) fn parse_whitelist_file(contents: &str) -> HashSet<String> {
    contents
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::WhitelistMode;

    // ── parse_whitelist_file ────────────────────────────────────────────────

    #[test]
    fn parse_strips_comments_and_blanks() {
        let contents = "# comment\naabbcc\n\n  ddeeff  \n# another comment\n112233\n";
        let set = parse_whitelist_file(contents);
        assert_eq!(set.len(), 3);
        assert!(set.contains("aabbcc"));
        assert!(set.contains("ddeeff"));
        assert!(set.contains("112233"));
    }

    #[test]
    fn parse_empty_file() {
        assert!(parse_whitelist_file("").is_empty());
        assert!(parse_whitelist_file("# only comments\n\n").is_empty());
    }

    // ── Whitelist::from_mode ────────────────────────────────────────────────

    // Open (no mode configured) — every pubkey is allowed.
    #[tokio::test]
    async fn open_allows_all() {
        // No DB needed; from_mode(None, _) ignores the db argument.
        // We use a dummy database URL — from_mode with None never touches the DB.
        let wl = Whitelist::default();
        assert!(wl.is_allowed("aabbcc").await);
        assert!(wl.is_allowed("000000").await);
    }

    // Static mode — only listed pubkeys are allowed.
    #[tokio::test]
    async fn static_allows_listed_pubkey() {
        let mode = WhitelistMode::Static(vec!["aabbcc".to_string()]);
        let wl = Whitelist::from_mode(Some(&mode), None);
        assert!(wl.is_allowed("aabbcc").await);
    }

    #[tokio::test]
    async fn static_denies_unlisted_pubkey() {
        let mode = WhitelistMode::Static(vec!["aabbcc".to_string()]);
        let wl = Whitelist::from_mode(Some(&mode), None);
        assert!(!wl.is_allowed("ddeeff").await);
    }

    #[tokio::test]
    async fn static_empty_list_denies_all() {
        let mode = WhitelistMode::Static(vec![]);
        let wl = Whitelist::from_mode(Some(&mode), None);
        assert!(!wl.is_allowed("aabbcc").await);
    }

    // File mode — starts with an empty in-memory set (watcher not running),
    // so every pubkey is denied until replace_all is called.
    #[tokio::test]
    async fn file_mode_starts_empty() {
        let mode = WhitelistMode::File("/nonexistent".into());
        let wl = Whitelist::from_mode(Some(&mode), None);
        assert!(!wl.is_allowed("aabbcc").await);
    }

    // replace_all (via file mode) correctly swaps the in-memory set.
    #[tokio::test]
    async fn file_mode_replace_all_updates_set() {
        let mode = WhitelistMode::File("/nonexistent".into());
        let wl = Whitelist::from_mode(Some(&mode), None);
        assert!(!wl.is_allowed("aabbcc").await);

        let new_set: HashSet<String> = ["aabbcc".to_string()].into();
        wl.replace_all(new_set).await;
        assert!(wl.is_allowed("aabbcc").await);
        assert!(!wl.is_allowed("ddeeff").await);
    }
}

async fn fallback_polling(
    this: Whitelist,
    path: PathBuf,
    mut last_modified: Option<SystemTime>,
    shutdown: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Stopping whitelist watcher (polling)");
                break;
            }
            _ = sleep(Duration::from_secs(10)) => {
                match tokio::fs::metadata(&path).await {
                    Ok(md) => match md.modified() {
                        Ok(modified) => {
                            let changed = last_modified.map_or(true, |prev| modified > prev);
                            if changed {
                                match tokio::fs::read_to_string(&path).await {
                                    Ok(contents) => {
                                        this.replace_all(parse_whitelist_file(&contents)).await;
                                        last_modified = Some(modified);
                                        info!("Reloaded whitelist from {}", path.display());
                                    }
                                    Err(e) => {
                                        warn!("Failed to read whitelist file {}: {}", path.display(), e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to get modification time for {}: {}", path.display(), e);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to stat whitelist file {}: {}", path.display(), e);
                    }
                }
            }
        }
    }
}

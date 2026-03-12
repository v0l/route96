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
#[derive(Clone, Default)]
pub struct Whitelist {
    inner: WhitelistInner,
}

#[derive(Clone, Default)]
enum WhitelistInner {
    /// Server is open; every pubkey is allowed.
    #[default]
    Open,
    /// Static in-memory set loaded from config, a file, or a file watcher update.
    /// Synchronisation is provided by the outer `Arc<RwLock<Whitelist>>` in `AppState`.
    InMemory(HashSet<String>),
    /// Entries live in the database, managed via the admin UI.
    Database(Database),
}

impl Whitelist {
    /// Build a [`Whitelist`] from the configured [`WhitelistMode`].
    ///
    /// - `None` → open server (no restrictions).
    /// - `Database` → DB-backed; `db` must be `Some`.
    /// - `Static(pubkeys)` → in-memory set.
    /// - `File(path)` → starts empty; call [`watch_file`] to load and hot-reload.
    pub fn from_mode(mode: Option<&WhitelistMode>, db: Option<&Database>) -> Self {
        match mode {
            None => Self::default(),
            Some(WhitelistMode::Database) => Self {
                inner: WhitelistInner::Database(
                    db.expect("Database whitelist mode requires a database connection")
                        .clone(),
                ),
            },
            Some(WhitelistMode::Static(pubkeys)) => Self {
                inner: WhitelistInner::InMemory(pubkeys.iter().cloned().collect()),
            },
            Some(WhitelistMode::File(_)) => {
                // Starts empty; the file watcher will populate it.
                Self {
                    inner: WhitelistInner::InMemory(HashSet::new()),
                }
            }
        }
    }

    /// Returns `true` if `pubkey_hex` is permitted to upload.
    pub async fn is_allowed(&self, pubkey_hex: &str) -> bool {
        match &self.inner {
            WhitelistInner::Open => true,
            WhitelistInner::InMemory(set) => set.contains(pubkey_hex),
            WhitelistInner::Database(db) => match db.whitelist_contains(pubkey_hex).await {
                Ok(found) => found,
                Err(e) => {
                    warn!("DB whitelist check failed, failing open: {}", e);
                    true
                }
            },
        }
    }

    /// Spawn a background task that loads `path` immediately and then watches
    /// it for changes, hot-reloading on every write.
    ///
    /// Should only be called when the mode is [`WhitelistMode::File`].
    /// The task writes updated sets back through `whitelist` so all request
    /// handlers see the new entries on their next `state.wl()` call.
    pub async fn watch_file(
        whitelist: Arc<RwLock<Whitelist>>,
        path: PathBuf,
        shutdown: CancellationToken,
    ) {
        let mut last_modified: Option<SystemTime> = None;
        info!("Starting whitelist watcher for {}", path.display());

        // Initial load
        if let Ok(md) = tokio::fs::metadata(&path).await
            && let Ok(modified) = md.modified()
            && let Ok(contents) = tokio::fs::read_to_string(&path).await
        {
            replace_set(&whitelist, parse_whitelist_file(&contents)).await;
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
                return fallback_polling(whitelist, path, last_modified, shutdown).await;
            }
        };

        let target_name = match path.file_name().map(|s| s.to_os_string()) {
            Some(n) => n,
            None => {
                warn!("Invalid whitelist file path: {}", path.display());
                return fallback_polling(whitelist, path, last_modified, shutdown).await;
            }
        };
        let watch_path = path.parent().map(|p| p.to_path_buf()).unwrap_or(path.clone());

        if let Err(e) = watcher.watch(&watch_path, RecursiveMode::NonRecursive) {
            warn!(
                "Falling back to polling: failed to watch {}: {}",
                path.display(),
                e
            );
            return fallback_polling(whitelist, path, last_modified, shutdown).await;
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
                                p.file_name()
                                    .map(|n| n == target_name.as_os_str())
                                    .unwrap_or(false)
                            });
                            if !relevant {
                                continue;
                            }
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
                        if let Some(t) = last_evt
                            && t.elapsed() < Duration::from_millis(250)
                        {
                            continue;
                        }
                        match tokio::fs::read_to_string(&path).await {
                            Ok(contents) => {
                                replace_set(&whitelist, parse_whitelist_file(&contents)).await;
                                info!("Reloaded whitelist from {} (debounced)", path.display());
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to read whitelist file {}: {}",
                                    path.display(),
                                    e
                                );
                            }
                        }
                        pending_change = false;
                    }
                }
            }
        }
    }
}

/// Swap the in-memory set inside `whitelist` for `new_set`.
/// No-ops if the current mode is not `InMemory`.
async fn replace_set(whitelist: &RwLock<Whitelist>, new_set: HashSet<String>) {
    if let WhitelistInner::InMemory(ref mut set) = whitelist.write().await.inner {
        *set = new_set;
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

    #[tokio::test]
    async fn open_allows_all() {
        let wl = Whitelist::default();
        assert!(wl.is_allowed("aabbcc").await);
        assert!(wl.is_allowed("000000").await);
    }

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

    #[tokio::test]
    async fn file_mode_starts_empty() {
        let mode = WhitelistMode::File("/nonexistent".into());
        let wl = Whitelist::from_mode(Some(&mode), None);
        assert!(!wl.is_allowed("aabbcc").await);
    }

    #[tokio::test]
    async fn file_mode_replace_set_updates_entries() {
        let mode = WhitelistMode::File("/nonexistent".into());
        let wl = Arc::new(RwLock::new(Whitelist::from_mode(Some(&mode), None)));
        assert!(!wl.read().await.is_allowed("aabbcc").await);

        let new_set: HashSet<String> = ["aabbcc".to_string()].into();
        replace_set(&wl, new_set).await;
        assert!(wl.read().await.is_allowed("aabbcc").await);
        assert!(!wl.read().await.is_allowed("ddeeff").await);
    }
}

async fn fallback_polling(
    whitelist: Arc<RwLock<Whitelist>>,
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
                            let changed = last_modified.is_none_or(|prev| modified > prev);
                            if changed {
                                match tokio::fs::read_to_string(&path).await {
                                    Ok(contents) => {
                                        replace_set(&whitelist, parse_whitelist_file(&contents)).await;
                                        last_modified = Some(modified);
                                        info!("Reloaded whitelist from {}", path.display());
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Failed to read whitelist file {}: {}",
                                            path.display(),
                                            e
                                        );
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to get modification time for {}: {}",
                                path.display(),
                                e
                            );
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

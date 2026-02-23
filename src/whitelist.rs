use log::{error, info, warn};
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};

#[derive(Clone, Default)]
pub struct Whitelist {
    list: Arc<RwLock<Option<HashSet<String>>>>,
}

impl Whitelist {
    pub fn new(initial: Option<Vec<String>>) -> Self {
        let set = initial.map(|v| v.into_iter().collect::<HashSet<_>>());
        Self {
            list: Arc::new(RwLock::new(set)),
        }
    }

    pub fn contains_hex(&self, pubkey_hex: &str) -> bool {
        let guard = self.list.read().unwrap();
        match &*guard {
            Some(set) => set.contains(pubkey_hex),
            None => true,
        }
    }

    fn replace_all(&self, new_list: Option<HashSet<String>>) {
        let mut guard = self.list.write().unwrap();
        *guard = new_list;
    }

    pub fn start_file_watcher(
        &self,
        path: PathBuf,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            let mut last_modified: Option<SystemTime> = None;
            info!("Starting whitelist watcher for {}", path.display());
            // initial load
            if let Ok(md) = tokio::fs::metadata(&path).await
                && let Ok(modified) = md.modified()
                && let Ok(contents) = tokio::fs::read_to_string(&path).await
            {
                let set: HashSet<String> = contents
                    .lines()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .map(|s| s.to_string())
                    .collect();
                this.replace_all(Some(set));
                last_modified = Some(modified);
                info!("Loaded whitelist from {}", path.display());
            }
            // Event-driven watching using notify; fallback to polling if it fails
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
                    warn!(
                        "Falling back to polling: failed to create file watcher: {}",
                        e
                    );
                    return fallback_polling(this, path, last_modified, shutdown_rx).await;
                }
            };
            let target_name = match path.file_name().map(|s| s.to_os_string()) {
                Some(n) => n,
                None => {
                    warn!("Invalid whitelist file path: {}", path.display());
                    return fallback_polling(this, path, last_modified, shutdown_rx).await;
                }
            };
            let watch_path = path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(path.clone());

            if let Err(e) = watcher.watch(&watch_path, RecursiveMode::NonRecursive) {
                warn!(
                    "Falling back to polling: failed to watch {}: {}",
                    path.display(),
                    e
                );
                return fallback_polling(this, path, last_modified, shutdown_rx).await;
            }
            let mut pending_change = false;
            let mut last_evt: Option<Instant> = None;
            let mut debounce = tokio::time::interval(Duration::from_millis(300));
            // first tick completes immediately; advance it to avoid immediate fire
            debounce.tick().await;
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Stopping whitelist watcher");
                        break;
                    }
                    Some(evt) = rx.recv() => {
                        match evt {
                            Ok(event) => {
                                // Only react to events that reference our target filename
                                let relevant = event.paths.iter().any(|p| p.file_name().map(|n| n==target_name.as_os_str()).unwrap_or(false));
                                if !relevant { continue; }
                                match event.kind {
                                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) | EventKind::Access(notify::event::AccessKind::Close(notify::event::AccessMode::Write)) => {
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
                            if let Some(t) = last_evt && t.elapsed() < Duration::from_millis(250) { continue; }
                            match tokio::fs::read_to_string(&path).await {
                                Ok(contents) => {
                                    let set: HashSet<String> = contents
                                        .lines()
                                        .map(|l| l.trim())
                                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                        .map(|s| s.to_string())
                                        .collect();
                                    this.replace_all(Some(set));
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
        })
    }
}

async fn fallback_polling(
    this: Whitelist,
    path: PathBuf,
    mut last_modified: Option<SystemTime>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Stopping whitelist watcher (polling)");
                break;
            }
            _ = sleep(Duration::from_secs(10)) => {
                match tokio::fs::metadata(&path).await {
                    Ok(md) => {
                        match md.modified() {
                            Ok(modified) => {
                                let changed = match last_modified {
                                    Some(prev) => modified > prev,
                                    None => true,
                                };
                                if changed {
                                    match tokio::fs::read_to_string(&path).await {
                                        Ok(contents) => {
                                            let set: HashSet<String> = contents
                                                .lines()
                                                .map(|l| l.trim())
                                                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                                .map(|s| s.to_string())
                                                .collect();
                                            this.replace_all(Some(set));
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
                        }
                    }
                    Err(e) => {
                        // If file doesn't exist or can't stat, keep previous list
                        warn!("Failed to stat whitelist file {}: {}", path.display(), e);
                    }
                }
            }
        }
    }
}

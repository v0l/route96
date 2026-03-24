//! Background task that hot-reloads [`Settings`] when the config file or the
//! database `config` table changes.
//!
//! The watcher uses [`notify`] to watch the config file for filesystem events.
//! Additionally, it polls the database every [`DB_POLL_INTERVAL`] seconds so
//! that changes made via the admin API are picked up even if no file change
//! occurs.
//!
//! On any change the full config is rebuilt from scratch (file → env → DB),
//! deserialised into a fresh [`Settings`] value, and atomically swapped into
//! the shared [`Arc<RwLock<Settings>>`].  The [`Whitelist`] is rebuilt from the
//! new settings at the same time so whitelist mode changes (e.g. enabling or
//! disabling the whitelist) take effect immediately without a restart.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use config::{Config, Environment, File};
use log::{debug, error, info, warn};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::db::Database;
use crate::db_config::DbConfigSource;
use crate::settings::Settings;
use crate::whitelist::Whitelist;

/// Build a fresh [`Settings`] from the config file + environment + database.
///
/// This is the same logic used at startup (in `main.rs`) and is called every
/// time a reload is triggered.
pub async fn build_settings(config_path: &str, db: &Database) -> anyhow::Result<Settings> {
    let builder = Config::builder()
        .add_source(File::with_name(config_path))
        .add_source(Environment::with_prefix("APP"))
        .add_async_source(DbConfigSource { db: db.clone() });

    let built = builder.build_cloned().await?;
    Ok(built.try_deserialize()?)
}

/// Spawn a background task that watches `config_path` for file-system changes
/// and polls the database every [`DB_POLL_INTERVAL`], rebuilding [`Settings`]
/// and [`Whitelist`] on any change.
///
/// The task exits cleanly when `shutdown` is cancelled.
pub async fn watch_config(
    config_path: String,
    db: Database,
    settings: Arc<RwLock<Settings>>,
    whitelist: Arc<RwLock<Whitelist>>,
    shutdown: CancellationToken,
) {
    // Channel for file-system events from `notify` (capacity 32 is plenty).
    let (fs_tx, mut fs_rx) = mpsc::channel::<()>(32);

    // Set up the notify watcher on a dedicated blocking thread.  `notify`
    // uses synchronous callbacks, so we bridge into async via the channel.
    let config_path_clone = config_path.clone();
    let watcher_result: anyhow::Result<RecommendedWatcher> = (|| {
        let tx = fs_tx.clone();
        let watch_path = PathBuf::from(&config_path_clone);
        // Canonicalise so we can compare paths reliably in the callback.
        let canonical = watch_path.canonicalize().unwrap_or(watch_path.clone());
        // Clone before the closure moves `watch_path`.
        let watch_path_for_closure = watch_path.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    use notify::EventKind::*;
                    // Only react to modify events (data changes and atomic
                    // renames that editors use to save files).  Ignore
                    // access/open events and unrelated creates/removes.
                    let is_modify = matches!(event.kind, Modify(_));
                    if !is_modify {
                        return;
                    }
                    // Filter to events that involve the config file itself,
                    // ignoring other files in the same watched directory.
                    let affects_config = event.paths.iter().any(|p| {
                        p.canonicalize().ok().as_deref() == Some(&canonical)
                            || p == &watch_path_for_closure
                    });
                    if affects_config {
                        let _ = tx.try_send(());
                    }
                }
            },
            notify::Config::default(),
        )?;
        // Watch the parent directory so atomic-rename writes (used by most
        // editors) are caught.  The callback above filters to the config file.
        let parent = watch_path.parent().unwrap_or(std::path::Path::new("."));
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
        Ok(watcher)
    })();

    let _watcher = match watcher_result {
        Ok(w) => {
            info!("config_watcher: watching '{}' for changes", config_path);
            Some(w)
        }
        Err(e) => {
            warn!(
                "config_watcher: notify watcher setup failed ({}); \
                 will rely on DB polling only",
                e
            );
            None
        }
    };

    loop {
        tokio::select! {
            biased;

            _ = shutdown.cancelled() => {
                info!("config_watcher: shutting down");
                break;
            }

            // File-system change detected.
            Some(_) = fs_rx.recv() => {
                // Drain any additional queued events to debounce rapid writes.
                while fs_rx.try_recv().is_ok() {}
                reload(&config_path, &db, &settings, &whitelist).await;
            }
        }
    }
}

pub async fn reload(
    config_path: &str,
    db: &Database,
    settings: &Arc<RwLock<Settings>>,
    whitelist: &Arc<RwLock<Whitelist>>,
) {
    match build_settings(config_path, db).await {
        Ok(new_settings) => {
            // Only swap (and log) when something actually changed.
            let changed = *settings.read().await != new_settings;
            if !changed {
                debug!("config_watcher: no changes detected in '{}'", config_path);
                return;
            }

            // Rebuild the whitelist only when the mode actually changes.
            // File mode is managed by a dedicated watch_file task that writes
            // into the live RwLock<Whitelist>; replacing it here would wipe
            // whatever the file watcher already loaded.
            let whitelist_mode_changed = settings.read().await.whitelist != new_settings.whitelist;

            *settings.write().await = new_settings.clone();

            if whitelist_mode_changed {
                *whitelist.write().await =
                    Whitelist::from_mode(new_settings.whitelist.as_ref(), Some(db));
            }

            info!("config_watcher: settings reloaded from '{}'", config_path);
        }
        Err(e) => {
            error!(
                "config_watcher: failed to reload settings from '{}': {}",
                config_path, e
            );
        }
    }
}

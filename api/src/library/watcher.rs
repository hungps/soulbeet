use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dioxus::logger::tracing::{info, warn};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::models::folder::Folder;

use super::{all_folders, scan_folder_internal, scan_settings};

struct WatcherHandle {
    _watcher: RecommendedWatcher,
    signature: Vec<(String, String)>,
}

pub async fn watch_loop() {
    let (tx, rx) = unbounded_channel::<String>();
    tokio::spawn(debounce_consumer(rx));

    let mut handle: Option<WatcherHandle> = None;
    loop {
        let settings = scan_settings().await;
        let folders = all_folders().await;
        let signature: Vec<(String, String)> = folders
            .iter()
            .map(|folder| (folder.id.clone(), folder.path.clone()))
            .collect();

        if !settings.watch_enabled {
            if handle.take().is_some() {
                info!("Library file watcher disabled");
            }
        } else {
            let needs_rebuild = handle
                .as_ref()
                .map(|handle| handle.signature != signature)
                .unwrap_or(true);
            if needs_rebuild {
                drop(handle.take());
                match build_watcher(tx.clone(), &folders) {
                    Ok(Some(watcher)) => {
                        info!("Library file watcher active on {} folder(s)", folders.len());
                        handle = Some(WatcherHandle {
                            _watcher: watcher,
                            signature,
                        });
                    }
                    Ok(None) => {
                        handle = None;
                    }
                    Err(e) => warn!("Could not start library file watcher: {}", e),
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

fn ignored_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    name == ".beets_library.db"
        || name.ends_with(".db-shm")
        || name.ends_with(".db-wal")
        || name.ends_with(".db-journal")
        || name.ends_with(".tmp")
        || name.ends_with(".part")
        || name.starts_with("._")
}

fn build_watcher(
    tx: UnboundedSender<String>,
    folders: &[Folder],
) -> Result<Option<RecommendedWatcher>, String> {
    if folders.is_empty() {
        return Ok(None);
    }
    let mappings: Arc<Vec<(PathBuf, String)>> = Arc::new(
        folders
            .iter()
            .map(|folder| (PathBuf::from(&folder.path), folder.id.clone()))
            .collect(),
    );

    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            return;
        };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        for path in event.paths {
            if ignored_path(&path) {
                continue;
            }
            if let Some((_, folder_id)) = mappings.iter().find(|(root, _)| path.starts_with(root)) {
                let _ = tx.send(folder_id.clone());
            }
        }
    })
    .map_err(|e| e.to_string())?;

    let mut watched = 0usize;
    for folder in folders {
        let path = Path::new(&folder.path);
        if !path.is_dir() {
            continue;
        }
        match watcher.watch(path, RecursiveMode::Recursive) {
            Ok(()) => watched += 1,
            Err(e) => warn!("Could not watch {}: {}", folder.path, e),
        }
    }

    if watched == 0 {
        return Ok(None);
    }
    Ok(Some(watcher))
}

async fn debounce_consumer(mut rx: UnboundedReceiver<String>) {
    let mut pending: HashMap<String, Instant> = HashMap::new();
    loop {
        if let Ok(folder_id) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            match folder_id {
                Some(folder_id) => {
                    pending.entry(folder_id).or_insert_with(Instant::now);
                }
                None => break,
            }
        }

        let ready: Vec<String> = pending
            .iter()
            .filter(|(_, first)| first.elapsed() >= Duration::from_secs(8))
            .map(|(folder_id, _)| folder_id.clone())
            .collect();
        for folder_id in ready {
            pending.remove(&folder_id);
            if let Err(e) = scan_folder_internal(&folder_id, false).await {
                info!("Watcher-triggered library scan failed: {}", e);
            }
        }
    }
}

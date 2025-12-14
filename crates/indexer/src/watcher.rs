use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mime_guess::from_path;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use ghostdrive_core::{FileMetadata, MediaHash, StreamError, StreamResult};
use tokio::sync::mpsc;
use tokio::time::{interval, Instant};
use tracing::{error, info, warn};

use crate::FileIndex;

/// Events user internally by the watcher loop
#[derive(Debug)]
enum WatcherEvent {
    FileSystem(Event),
    ScanTick,
}

pub struct FileWatcher {
    index: Arc<FileIndex>,
    // Keep watcher alive by holding it, even if we don't access it directly after init
    _watcher: RecommendedWatcher,
    event_rx: mpsc::UnboundedReceiver<WatcherEvent>,
}

impl FileWatcher {
    pub fn new(index: Arc<FileIndex>, watch_paths: Vec<PathBuf>) -> StreamResult<Self> {
        let (tx, rx) = mpsc::unbounded_channel();

        // Proxy notify events to tokio channel
        let tx_clone = tx.clone();
        let watcher_handler = move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    let _ = tx_clone.send(WatcherEvent::FileSystem(event));
                }
                Err(e) => {
                    error!("File watcher error: {}", e);
                }
            }
        };

        let mut watcher = RecommendedWatcher::new(watcher_handler, Config::default())
            .map_err(|e| StreamError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        for path in &watch_paths {
            if !path.exists() {
                fs::create_dir_all(path).map_err(StreamError::Io)?;
            }
            watcher.watch(path, RecursiveMode::Recursive)
                .map_err(|e| StreamError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            info!("Watching path: {:?}", path);
        }

        // Set up a ticker for debouncing check
        let tx_tick = tx;
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(200));
            loop {
                ticker.tick().await;
                if tx_tick.send(WatcherEvent::ScanTick).is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            index,
            _watcher: watcher,
            event_rx: rx,
        })
    }

    /// Main loop processing events with debouncing
    pub async fn run(mut self) -> StreamResult<()> {
        info!("FileWatcher started");

        // Map path -> Instant (when to process)
        let mut pending_updates: HashMap<PathBuf, Instant> = HashMap::new();
        let debounce_duration = Duration::from_millis(500);

        while let Some(event) = self.event_rx.recv().await {
            match event {
                WatcherEvent::FileSystem(fs_event) => {
                    self.handle_fs_event(fs_event, &mut pending_updates, debounce_duration);
                }
                WatcherEvent::ScanTick => {
                    self.process_pending(&mut pending_updates).await;
                }
            }
        }

        Ok(())
    }

    fn handle_fs_event(
        &self,
        event: Event,
        pending: &mut HashMap<PathBuf, Instant>,
        debounce: Duration
    ) {
        for path in event.paths {
            if self.should_ignore(&path) {
                continue;
            }

            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    // Schedule for update
                    pending.insert(path, Instant::now() + debounce);
                }
                EventKind::Remove(_) => {
                    // Remove immediately
                    pending.remove(&path);
                    if let Err(e) = self.index.remove_file(&path) {
                        error!("Failed to remove file from index: {}", e);
                    } else {
                        info!("File removed: {:?}", path);
                    }
                }
                _ => {}
            }
        }
    }

    async fn process_pending(&self, pending: &mut HashMap<PathBuf, Instant>) {
        let now = Instant::now();
        let mut to_process = Vec::new();

        // Identify keys to process
        // (cloning paths to avoid borrow checker issues during iteration)
        let keys: Vec<PathBuf> = pending.keys().cloned().collect();
        for path in keys {
            if let Some(deadline) = pending.get(&path) {
                if now >= *deadline {
                    to_process.push(path.clone());
                    pending.remove(&path);
                }
            }
        }

        // Process ready files
        for path in to_process {
            let index = self.index.clone();

            // Spawn blocking task for heavy IO/Hashing
            tokio::task::spawn_blocking(move || {
                if let Err(e) = process_file_blocking(&index, path) {
                    warn!("Failed to process file: {}", e);
                }
            });
        }
    }

    fn should_ignore(&self, path: &Path) -> bool {
        // Ignore hidden files (Unix style)
        if let Some(name) = path.file_name() {
            if let Some(s) = name.to_str() {
                if s.starts_with('.') {
                    return true;
                }
            }
        }
        false
    }
}

/// Helper function to hash and metadata a file (Blocking IO)
fn process_file_blocking(index: &FileIndex, path: PathBuf) -> StreamResult<()> {
    // Re-check existence as it might have been deleted during debounce
    if !path.exists() || !path.is_file() {
        return Ok(());
    }

    let metadata = fs::metadata(&path).map_err(StreamError::Io)?;
    let size = metadata.len();

    // Hash content
    let file = fs::File::open(&path).map_err(StreamError::Io)?;
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut reader, &mut hasher).map_err(StreamError::Io)?;
    let hash_bytes = hasher.finalize();
    let hash = MediaHash(hash_bytes.to_hex().to_string());

    // Detect Mime
    let mime_type = from_path(&path).first_or_octet_stream().to_string();

    // Get creation time
    let created_at = metadata.created()
        .unwrap_or(SystemTime::now())
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let meta = FileMetadata {
        path: path.clone(),
        hash,
        size,
        mime_type,
        created_at,
    };

    index.upsert_file(&meta)?;
    info!("Indexed file: {:?} (Size: {} bytes)", path, size);

    Ok(())
}

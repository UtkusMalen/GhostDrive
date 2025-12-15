use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use ghostdrive_core::{FileMetadata, MediaHash, StreamError, StreamResult};
use ghostdrive_indexer::{FileIndex, FileWatcher};
use ghostdrive_network::StreamNode;
use ghostdrive_transcoder::TranscodeOptions;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};
use async_recursion::async_recursion;

pub struct HostConfig {
    pub data_dir: PathBuf,
    pub watch_paths: Vec<PathBuf>,
    pub transcode_options: TranscodeOptions
}

pub struct HostDaemon {
    index: Arc<FileIndex>,
    node: Arc<StreamNode>,
    config: HostConfig,
    _watcher_handle: JoinHandle<()>,
    shutdown_token: CancellationToken,
}

impl HostDaemon {
    pub async fn new(config: HostConfig) -> StreamResult<Self> {
        info!("Initializing Host Daemon...");

        // Initialize components
        let db_path = config.data_dir.join("index.db");
        let index = Arc::new(FileIndex::open(db_path)?);

        // Initialize node (handles identity and Iroh connection)
        let node = Arc::new(StreamNode::new(config.data_dir.clone()).await?);

        // Start FileWatcher
        let watcher_index = index.clone();
        let watch_paths = config.watch_paths.clone();

        // Start watcher in background
        // Watcher currently manages its own internal loop, so we wrap it
        let watcher = FileWatcher::new(watcher_index, watch_paths.clone())?;

        let shutdown_token = CancellationToken::new();
        let child_token = shutdown_token.clone();

        let watcher_handle = tokio::spawn(async move {
            tokio::select! {
                res = watcher.run() => {
                    if let Err(e) = res {
                        error!("FileWatcher crashed: {}", e);
                    }
                }
                _ = child_token.cancelled() => {
                    info!("FileWatcher shutting down via token");
                }
            }
        });

        let daemon = Self {
            index,
            node,
            config,
            _watcher_handle: watcher_handle,
            shutdown_token
        };

        // Initial Ingestion
        // Scan watch paths to ensure both Index and Node are up to date
        daemon.ingest_existing_files().await?;

        info!("Host daemon started successfully. Node ID: {}", daemon.node.node_id());
        Ok(daemon)
    }

    /// Perform a recursive scan of watch paths to register files
    async fn ingest_existing_files(&self) -> StreamResult<()> {
        info!("Starting initial ingestion scan...");
        for path in &self.config.watch_paths {
            if path.exists() {
                self.scan_recursive(path).await?;
            }
        }
        info!("Ingestion complete");
        Ok(())
    }

    #[async_recursion]
    async fn scan_recursive(&self, dir: &Path) -> StreamResult<()> {
        let mut entries = tokio::fs::read_dir(dir).await.map_err(StreamError::Io)?;

        while let Some(entry) = entries.next_entry().await.map_err(StreamError::Io)? {
            let path = entry.path();
            if path.is_dir() {
                self.scan_recursive(&path).await?;
            } else {
                // Process file
                if let Err(e) = self.register_file(&path).await {
                    warn!("Failed to ingest {:?}: {}", path, e);
                }
            }
        }
        Ok(())
    }

    /// Helper to register a file with both Iroh (Node) and Redb (Index)
    async fn register_file(&self, path: &PathBuf) -> StreamResult<MediaHash> {
        // Add to Iroh Node (computes/verifies hash)
        // Using node to get the hash first, as it's the source of truth for network
        let hash = self.node.add_file_reference(path.clone()).await?;

        // Gather metadata
        let metadata = tokio::fs::metadata(path).await.map_err(StreamError::Io)?;
        let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
        let created_at = metadata.created()
            .unwrap_or(SystemTime::now())
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let meta = FileMetadata {
            path: path.clone(),
            hash: hash.clone(),
            size: metadata.len(),
            mime_type: mime,
            created_at
        };

        // Update index
        self.index.upsert_file(&meta)?;

        Ok(hash)
    }

    /// Share a specific file by path
    #[instrument(skip(self))]
    pub async fn share_file(&self, path: PathBuf) -> StreamResult<String> {
        let canonical = path.canonicalize().map_err(StreamError::Io)?;

        // Ensure file is ready in Iroh
        let hash = self.register_file(&canonical).await?;

        let file_name = canonical.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let ticket = self.node.generate_ticket(hash, file_name);

        Ok(ticket.encode())
    }

    /// Share a folder as a collection
    pub async fn share_folder(&self, path: PathBuf) -> StreamResult<String> {
        let canonical = path.canonicalize().map_err(StreamError::Io)?;

        if !canonical.is_dir() {
            return Err(StreamError::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "Path is not a directory"
            )));
        }

        // Collect all file hashes in the folder (flat list for now)
        let mut hashes = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&canonical).await.map_err(StreamError::Io)?;

        while let Some(entry) = read_dir.next_entry().await.map_err(StreamError::Io)? {
            let entry_path = entry.path();
            if entry_path.is_file() {
                // Ensure registered
                let hash = self.register_file(&entry_path).await?;
                hashes.push(hash);
            }
        }

        if hashes.is_empty() {
            return Err(StreamError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No files found in directory"
            )));
        }

        // Create collection
        let collection_hash = self.node.create_collection(hashes).await?;

        let folder_name = canonical.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "collection".to_string());

        let ticket = self.node.generate_ticket(collection_hash, folder_name);

        Ok(ticket.encode())
    }

    /// Get reference to the node
    pub fn node(&self) -> Arc<StreamNode> {
        self.node.clone()
    }
}

impl Drop for HostDaemon {
    fn drop(&mut self) {
        // Signal watcher to stop
        self.shutdown_token.cancel();
    }
}
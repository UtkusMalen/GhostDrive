use std::path::PathBuf;
use std::time::Duration;

use ghostdrive_core::{MediaHash, ShareTicket, StreamError, StreamResult};
use iroh::{Endpoint, EndpointId, SecretKey};
use iroh::protocol::Router;
use iroh_blobs::{
    BlobsProtocol,
    store::fs::FsStore as BlobStore,
    api::blobs::{AddPathOptions, ImportMode},
    BlobFormat, Hash, ALPN,
};
use tokio::fs;
use tracing::{info, warn};
use std::str::FromStr;

pub struct StreamNode {
    endpoint: Endpoint,
    store: BlobStore,
    _router: Router, // Keep router alive
    #[allow(dead_code)] // Kept for potential future use/export
    secret_key: SecretKey,
}

impl StreamNode {
    /// Initialize the Iroh node with persistent identity
    pub async fn new(data_dir: PathBuf) -> StreamResult<Self> {
        // Ensure data directory exists
        if !data_dir.exists() {
            fs::create_dir_all(&data_dir)
                .await
                .map_err(StreamError::Io)?;
        }

        let key_path = data_dir.join("secret.key");

        // Load or generate secret key
        let secret_key = if key_path.exists() {
            let hex_str = fs::read_to_string(&key_path)
                .await
                .map_err(StreamError::Io)?;
            let bytes = hex::decode(hex_str.trim())
                .map_err(|e| StreamError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
            SecretKey::from_bytes(&bytes.try_into().map_err(|_| {
                StreamError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid key length"))
            })?)
        } else {
            info!("Generating new persistent identity...");
            let key = SecretKey::generate(&mut rand::rng());
            let hex_str = hex::encode(key.to_bytes());

            fs::write(&key_path, hex_str).await.map_err(StreamError::Io)?;

            // Set permissions to 600 (Unix only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&key_path).await.map_err(StreamError::Io)?.permissions();
                perms.set_mode(0o600);
                fs::set_permissions(&key_path, perms).await.map_err(StreamError::Io)?;
            }

            key
        };

        // Initialize Blob Store
        let blobs_dir = data_dir.join("blobs");
        fs::create_dir_all(&blobs_dir)
            .await
            .map_err(StreamError::Io)?;

        let store = BlobStore::load(&blobs_dir)
            .await
            .map_err(|e| StreamError::Database(format!("Failed to load blob store: {}", e)))?;
            
        // Initialize Endpoint
        let endpoint = Endpoint::builder()
            .secret_key(secret_key.clone())
            .bind()
            .await
            .map_err(|e| StreamError::Iroh(e.to_string()))?;

        // Setup protocol router (Handling Blobs ALPN)
        let blobs_protocol = BlobsProtocol::new(&store, None); // Use reference and None events
        let router = Router::builder(endpoint.clone())
            .accept(ALPN, blobs_protocol)
            .spawn();

        // Log node details
        info!("GhostDrive Node Started");
        info!("  Node ID: {}", endpoint.id());

        // Wait briefly for the node to come online
        let _ = tokio::time::timeout(Duration::from_millis(500), async {
            let _ = endpoint.online().await;
        }).await;

        // Retrieve address information to find the relay URL
        let addr = endpoint.addr();
        if let Some(relay) = addr.relay_urls().next() {
            info!("  Relay URL: {}", relay);
        } else {
            warn!("  Relay URL: Pending/Unknown");
        }

        Ok(Self {
            endpoint,
            store,
            _router: router,
            secret_key,
        })
    }

    /// Return the base32-encoded Node ID
    pub fn node_id(&self) -> String {
        self.endpoint.id().to_string()
    }

    /// Return the EndpointId object
    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// Get the primary relay URL (if connected)
    pub fn relay_url(&self) -> String {
        self.endpoint
            .addr()
            .relay_urls()
            .next()
            .map(|r| r.to_string())
            .unwrap_or_else(|| "None".to_string())
    }

    /// Get a reference to the underlying Iroh Endpoint
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Add a file to the blob store using path reference (no copy)
    pub async fn add_file_reference(
        &self,
        file_path: PathBuf
    ) -> Result<MediaHash, StreamError> {
        if !file_path.exists() {
            return Err(StreamError::FileNotFound(file_path));
        }

        let options = AddPathOptions {
            path: file_path.clone(),
            mode: ImportMode::TryReference,
            format: BlobFormat::Raw,
        };

        // Import file into store without copying (TryReference)
        // .await on AddProgress yields the final result (RequestResult<TagInfo>)
        let outcome = self.store.add_path_with_opts(options)
            .await
            .map_err(|e| StreamError::Iroh(format!("Failed to add file reference: {}", e)))?;

        let hash = outcome.hash;
        info!("Added file reference: {:?} (Hash: {})", file_path, hash);

        Ok(MediaHash(hash.to_string()))
    }

    /// Create a collection (HashSeq) from multiple file hashes
    pub async fn create_collection(
        &self,
        hashes: Vec<MediaHash>
    ) -> Result<MediaHash, StreamError> {
        // Convert MediaHash strings to iroh::Hash
        let blob_hashes: Result<Vec<Hash>, _> = hashes.iter()
            .map(|h| Hash::from_str(&h.0))
            .collect();

        let blob_hashes = blob_hashes.map_err(|e| StreamError::InvalidHash(e.to_string()))?;

        // Create collection manually (list of 32-byte hashes)
        let mut bytes = Vec::with_capacity(blob_hashes.len() * 32);
        for h in blob_hashes {
            bytes.extend_from_slice(h.as_bytes());
        }

        // Add the collection blob itself
        let outcome = self.store.add_bytes(bytes)
            .await
            .map_err(|e| StreamError::Iroh(format!("Failed to create collection: {}", e)))?;

        let hash = outcome.hash;
        info!("Created collection with hash: {}", hash);

        Ok(MediaHash(hash.to_string()))
    }

    /// Generate a shareable ticket
    pub fn generate_ticket(
        &self,
        hash: MediaHash,
        name: String
    ) -> ShareTicket {
        ShareTicket {
            node_id: self.node_id(),
            relay_url: self.relay_url(),
            hash,
            name,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}
use std::path::PathBuf;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use ghostdrive_core::{FileMetadata, MediaHash, StreamError, StreamResult};
use tracing::{debug, info};

/// Table: File Path (String) -> Serialized FileMetadata (Bytes)
const FILES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("files");

/// Table: Content Hash (String) -> File Path (String)
const HASH_INDEX: TableDefinition<&str, &str> = TableDefinition::new("hash_index");

pub struct FileIndex {
    db: Database
}

impl FileIndex {
    /// Open or create the index database at the specified path
    pub fn open(path: PathBuf) -> StreamResult<Self> {
        info!("Opening database at: {:?}", path);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StreamError::Io)?;
        }

        let db = Database::create(&path).map_err(|e| StreamError::Database(e.to_string()))?;

        // Verify we can open write transaction
        let txn = db.begin_write().map_err(|e| StreamError::Database(e.to_string()))?;
        {
            // Just opening the table initializes them
            let _ = txn.open_table(FILES_TABLE).map_err(|e| StreamError::Database(e.to_string()))?;
            let _ = txn.open_table(HASH_INDEX).map_err(|e| StreamError::Database(e.to_string()))?;
        }
        txn.commit().map_err(|e| StreamError::Database(e.to_string()))?;

        Ok(Self { db })
    }

    /// Insert or update a file's metadata
    pub fn upsert_file(&self, metadata: &FileMetadata) -> StreamResult<()> {
        let path_str = metadata.path.to_string_lossy();
        let hash_str = metadata.hash.0.as_str();

        // Serialize FileMetadata
        let config = bincode::config::standard();
        let encoded = bincode::serde::encode_to_vec(metadata, config)
            .map_err(|e| StreamError::Database(format!("Serialization error: {}", e)))?;

        let txn = self.db.begin_write()
            .map_err(|e| StreamError::Database(e.to_string()))?;

        {
            let mut files_table = txn.open_table(FILES_TABLE)
                .map_err(|e| StreamError::Database(e.to_string()))?;
            let mut hash_table = txn.open_table(HASH_INDEX)
                .map_err(|e| StreamError::Database(e.to_string()))?;

            // Insert into FILES_TABLE (Path -> Metadata)
            files_table.insert(path_str.as_ref(), encoded.as_slice())
                .map_err(|e| StreamError::Database(e.to_string()))?;

            // Insert into HASH_INDEX (Hash -> Path)
            hash_table.insert(hash_str, path_str.as_ref())
                .map_err(|e| StreamError::Database(e.to_string()))?;
        }

        txn.commit().map_err(|e| StreamError::Database(e.to_string()))?;

        debug!("Inserted file: {:?}", metadata.path);
        Ok(())
    }

    /// Get file metadata by path
    pub fn get_by_path(&self, path: &std::path::Path) -> StreamResult<Option<FileMetadata>> {
        let txn = self.db.begin_read()
            .map_err(|e| StreamError::Database(e.to_string()))?;

        let files_table = txn.open_table(FILES_TABLE)
            .map_err(|e| StreamError::Database(e.to_string()))?;

        let path_str = path.to_string_lossy();

        if let Some(access) = files_table.get(path_str.as_ref())
            .map_err(|e| StreamError::Database(e.to_string()))?
        {
            let config = bincode::config::standard();
            // bincode::serde::decode_from_slice return (T, usize)
            let (metadata, _): (FileMetadata, usize) = bincode::serde::decode_from_slice(access.value(), config)
                .map_err(|e| StreamError::Database(format!("Deserialization error: {}", e)))?;

            Ok(Some(metadata))
        } else {
            Ok(None)
        }
    }

    /// Get file metadata by hash (reverse lookup)
    pub fn get_by_hash(&self, hash: &MediaHash) -> StreamResult<Option<FileMetadata>> {
        let txn = self.db.begin_read()
            .map_err(|e| StreamError::Database(e.to_string()))?;

        let hash_table = txn.open_table(HASH_INDEX)
            .map_err(|e| StreamError::Database(e.to_string()))?;

        let files_table = txn.open_table(FILES_TABLE)
            .map_err(|e| StreamError::Database(e.to_string()))?;

        // Lookup path in HASH_INDEX
        if let Some(path_access) = hash_table.get(hash.0.as_str())
            .map_err(|e| StreamError::Database(e.to_string()))?
        {
            let path_str = path_access.value();

            // Query FILES_TABLE
            if let Some(file_access) = files_table.get(path_str)
                .map_err(|e| StreamError::Database(e.to_string()))?
            {
                let config = bincode::config::standard();
                let (metadata, _): (FileMetadata, usize) = bincode::serde::decode_from_slice(file_access.value(), config)
                    .map_err(|e| StreamError::Database(format!("Deserialization error: {}", e)))?;

                return Ok(Some(metadata));
            }
        }

        Ok(None)
    }

    /// Remove a file from index
    pub fn remove_file(&self, path: &std::path::Path) -> StreamResult<()> {
        let txn = self.db.begin_write()
            .map_err(|e| StreamError::Database(e.to_string()))?;

        let path_str = path.to_string_lossy();

        // Need to retrieve metadata first to find the hash for the reverse index
        let hash_to_remove = {
            let files_table = txn.open_table(FILES_TABLE)
                .map_err(|e| StreamError::Database(e.to_string()))?;

            if let Some(access) = files_table.get(path_str.as_ref())
                .map_err(|e| StreamError::Database(e.to_string()))?

            {
                let config = bincode::config::standard();
                let (meta, _): (FileMetadata, usize) = bincode::serde::decode_from_slice(access.value(), config)
                    .map_err(|e| StreamError::Database(format!("Deserialization error: {}", e)))?;
                Some(meta.hash)
            } else {
                None
            }
        };

        {
            let mut files_table = txn.open_table(FILES_TABLE)
                .map_err(|e| StreamError::Database(e.to_string()))?;
            let mut hash_table = txn.open_table(HASH_INDEX)
                .map_err(|e| StreamError::Database(e.to_string()))?;

            // Remove from files table
            files_table.remove(path_str.as_ref())
                .map_err(|e| StreamError::Database(e.to_string()))?;

            // Remove from hash index
            if let Some(hash) = hash_to_remove {
                hash_table.remove(hash.0.as_str())
                    .map_err(|e| StreamError::Database(e.to_string()))?;
            }
        }

        txn.commit().map_err(|e| StreamError::Database(e.to_string()))?;
        debug!("Removed file: {:?}", path);
        Ok(())
    }

    /// List all indexed files
    pub fn list_all(&self) -> StreamResult<Vec<FileMetadata>> {
        let txn = self.db.begin_read()
            .map_err(|e| StreamError::Database(e.to_string()))?;

        let files_table = txn.open_table(FILES_TABLE)
            .map_err(|e| StreamError::Database(e.to_string()))?;

        let mut results = Vec::new();
        let limit = 10_000;
        let config = bincode::config::standard();

        for (i, entry) in files_table.iter().map_err(|e| StreamError::Database(e.to_string()))?.enumerate() {
            if i >= limit {
                break;
            }
            let (_, value) = entry.map_err(|e| StreamError::Database(e.to_string()))?;
            let (metadata, _): (FileMetadata, usize) = bincode::serde::decode_from_slice(value.value(), config)
                .map_err(|e| StreamError::Database(format!("Deserialization error: {}", e)))?;
            results.push(metadata);
        }

        Ok(results)
    }

    /// Compact the database to reclaim free space
    /// Returns true if compaction was performed
    pub fn compact(&mut self) -> StreamResult<bool> {
        self.db
            .compact()
            .map_err(|e| StreamError::Database(e.to_string()))
    }
}
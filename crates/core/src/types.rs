use serde::{Deserialize, Serialize};
use std::fmt::Formatter;
use std::path::PathBuf;

/// Wrapper for content hashes (BLAKE3) used by Iroh
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MediaHash(pub String);

impl std::fmt::Display for MediaHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for MediaHash {
    fn from(s: String) -> Self {
        MediaHash(s)
    }
}

/// Metadata for indexed media files
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileMetadata {
    /// Absolute path to the file on the host
    pub path: PathBuf,
    /// Content hash (BLAKE3)
    pub hash: MediaHash,
    /// File size in bytes
    pub size: u64,
    /// MIME type (e.g., "video/mp4")
    pub mime_type: String,
    /// Unix timestamp of file creation/modification
    pub created_at: u64,
}
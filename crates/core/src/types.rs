use crate::error::StreamError;
use serde::{Deserialize, Serialize};
use std::fmt::Formatter;
use std::path::PathBuf;
use base64::prelude::*;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareTicket {
    pub node_id: String,
    pub relay_url: String,
    pub hash: MediaHash,
    pub name: String, // File or collection name
    pub created_at: u64,
}

impl ShareTicket {
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).expect("ShareTicket serialization error");
        BASE64_STANDARD.encode(json)
    }

    pub fn decode(ticket: &str) -> Result<Self, StreamError> {
        let bytes = BASE64_STANDARD
            .decode(ticket)
            .map_err(|e| StreamError::InvalidHash(format!("Base64 decode failed: {}", e)))?;

        let ticket: ShareTicket = serde_json::from_slice(&bytes)
            .map_err(|e| StreamError::InvalidHash(format!("JSON decode failed: {}", e)))?;

        Ok(ticket)
    }
}
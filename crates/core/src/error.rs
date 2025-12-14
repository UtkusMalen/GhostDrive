use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StreamError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Iroh networking error: {0}")]
    Iroh(String),

    #[error("Transcoding error: {0}")]
    Transcode(String),

    #[error("Invalid hash: {0}")]
    InvalidHash(String),

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("Not connected to peer")]
    NotConnected,
}

// Result type alias
pub type StreamResult<T> = Result<T, StreamError>;
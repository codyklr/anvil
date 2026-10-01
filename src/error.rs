//! Typed error surface for Anvil.
//!
//! Errors are split between ones a user can recover from (e.g. malformed rows)
//! and ones that fail an entire open (e.g. archive corruption).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnvilError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("unsupported compression method {0:?} for entry {1:?}")]
    UnsupportedCompression(String, String),

    #[error("archive entry {0:?} is encrypted; Anvil cannot open password-protected entries")]
    EncryptedEntry(String),

    #[error("archive contains no supported data files")]
    NoUsableEntries,

    #[error("archive entries needed but {0} was requested")]
    EntryNotFound(String),

    #[error("path traversal attempt in archive entry: {0}")]
    PathTraversal(String),

    #[error("decompressed size {actual} exceeds limit {limit} for entry {name:?}")]
    DecompressedTooLarge {
        name: String,
        actual: u64,
        limit: u64,
    },

    #[error("empty file")]
    EmptyFile,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, AnvilError>;

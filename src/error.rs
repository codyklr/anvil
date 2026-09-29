//! Typed error surface for Anvil.
//!
//! Errors are split between ones a user can recover from (e.g. malformed rows)
//! and ones that fail an entire open (e.g. archive corruption).

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnvilError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("file is not a zip archive")]
    NotAZip(PathBuf),

    #[error("unsupported compression method {0:?} for entry {1:?}")]
    UnsupportedCompression(String, String),

    #[error("archive contains no supported data files")]
    NoUsableEntries,

    #[error("archive contains multiple entries; please select one ({0} candidates)")]
    AmbiguousArchive(usize),

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

    #[error("binary file: no newline bytes in first {sampled} bytes")]
    LooksBinary { sampled: usize },

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, AnvilError>;

//! DataSource abstraction.
//!
//! Anvil can read data from many physical origins. The rest of the code
//! never cares whether the bytes came from a regular file or a member of a
//! zip archive. A `DataSource` is the high-level description of the input.
//! An `OpenedSource` is the resolved physical artifact that the parser
//! indexes and the parser asking "give me record at offset X" can use.

pub mod archive;
pub mod local;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{AnvilError, Result};

/// How a single input was opened. Displayed in the UI.
#[derive(Debug, Clone)]
pub enum SourceKind {
    LocalFile,
    ArchiveEntry {
        archive_name: String,
        entry_name: String,
        compressed_size: u64,
        uncompressed_size: u64,
    },
}

/// High-level description of an input file.
#[derive(Debug, Clone)]
pub struct DataSource {
    pub kind: SourceKind,
    /// Primary location. For `LocalFile`, this is the file path. For
    /// `ArchiveEntry`, this is the containing archive path.
    pub path: PathBuf,
}

/// The fully-resolved physical artifact: an open file in the filesystem that
/// can be memory-mapped or seeked into.
///
/// `Arc<MmapSource>` is shared; cheap to clone because we only carry a
/// pointer to the mmap and its metadata.
pub struct MmapSource {
    /// Path actually used to read bytes (may be a temp file).
    pub path: PathBuf,
    mmap: Arc<memmap2::Mmap>,
    pub total_size: u64,
}

impl MmapSource {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path)?;
        let total_size = file.metadata()?.len();
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Ok(Self {
            path,
            mmap: Arc::new(mmap),
            total_size,
        })
    }

    /// Borrow the entire byte buffer.
    pub fn as_bytes(&self) -> &[u8] {
        &self.mmap[..]
    }
}

impl std::fmt::Debug for MmapSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MmapSource")
            .field("path", &self.path)
            .field("total_size", &self.total_size)
            .field("mmap_ptr", &Arc::as_ptr(&self.mmap))
            .finish()
    }
}

/// Result of resolving a `DataSource` to something parsable.
#[derive(Debug)]
pub struct OpenedSource {
    pub source: DataSource,
    pub physical: Arc<MmapSource>,
    /// Working/cached candidates. For archive entries we return one
    /// of these even though only one is selected; this gives the UI a
    /// way to switch without re-opening the archive.
    pub archive_candidates: Option<Vec<ArchiveCandidate>>,
    /// If we materialised the dataset into a temp file (e.g. extracted
    /// from a ZIP), this keeps the temp directory alive until the
    /// `OpenedSource` drops, which removes it.
    pub temp_dir: Option<Arc<tempfile::TempDir>>,
}

/// A potential data file inside an archive.
#[derive(Debug, Clone)]
pub struct ArchiveCandidate {
    pub entry_name: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    /// Whether we think this is a real data file (passes the heuristic
    /// filters: not __MACOSX, not .DS_Store, looks text-y, etc.)
    pub usable: bool,
    /// Preference among usable entries; lower is more data-like. `0` is a
    /// recognised tabular extension (csv/tsv/...), `2` is generic text.
    /// Ignored when `usable` is false.
    pub rank: u8,
    /// Why it was filtered out, if `usable` is false. Empty when usable.
    pub rejected_reason: String,
}

impl OpenedSource {
    /// Returns the bytes of the actually-selected entry.
    pub fn bytes(&self) -> &[u8] {
        self.physical.as_bytes()
    }

    /// Total byte length of the resolved (possibly decompressed) source.
    pub fn len(&self) -> u64 {
        self.physical.total_size
    }

    pub fn is_empty(&self) -> bool {
        self.physical.total_size == 0
    }

    pub fn display_label(&self) -> String {
        match &self.source.kind {
            SourceKind::LocalFile => self
                .source
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string(),
            SourceKind::ArchiveEntry {
                archive_name,
                entry_name,
                ..
            } => format!("{archive_name} :: {entry_name}"),
        }
    }
}

/// Open a single file path. If the path is a zip archive, returns the
/// list of candidates so the caller can choose.
pub fn open_path(path: impl AsRef<Path>) -> Result<OpenResult> {
    let path = path.as_ref();
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(AnvilError::Other(format!("{path:?} is not a regular file")));
    }

    // Peek at just the magic bytes to decide whether this is a zip.
    // Reading the whole file here would copy a multi-GB dataset into RAM.
    let mut magic = [0u8; 4];
    {
        use std::io::Read;
        let mut f = std::fs::File::open(path)?;
        let read = f.read(&mut magic)?;
        if read < 4 {
            // Fewer than 4 bytes: an empty or near-empty file.
            if read == 0 {
                return Err(AnvilError::EmptyFile);
            }
        }
    }
    // ZIP signatures: local file header (has entries) or end-of-central-
    // directory (empty archive). Anything else is treated as delimited text.
    let is_zip = magic == [0x50, 0x4b, 0x03, 0x04] || magic == [0x50, 0x4b, 0x05, 0x06];
    if is_zip {
        return archive::ArchiveSource::from_path(path).map(OpenResult::Archive);
    }

    let physical = MmapSource::open(path)?;
    Ok(OpenResult::Direct(OpenedSource {
        source: DataSource {
            kind: SourceKind::LocalFile,
            path: path.to_path_buf(),
        },
        physical: Arc::new(physical),
        archive_candidates: None,
        temp_dir: None,
    }))
}

/// What `open_path` returns: either a single resolved dataset, or an
/// archive with multiple candidates that need user selection.
#[derive(Debug)]
pub enum OpenResult {
    Direct(OpenedSource),
    Archive(archive::ArchiveSource),
}

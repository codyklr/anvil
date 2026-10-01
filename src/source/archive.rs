//! ZIP archive handling.
//!
//! `ArchiveSource` represents an open ZIP archive. Its main jobs are:
//!  - discover candidate data files
//!  - validate each entry (no zip-slip, sane sizes)
//!  - on demand, decompress the chosen entry to a temp file and return an
//!    `OpenedSource` for the parser to index
//!
//! Direct streaming of a ZIP entry through the parser is possible but
//! complicated by quoted multi-line records which need random access. For
//! nav after initial open we transparently extract to a temp file and
//! memory-map it, the same as a regular file.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use zip::read::ZipFile;
use zip::{CompressionMethod, ZipArchive};

use crate::error::{AnvilError, Result};
use crate::progress::{LoadPhase, LoadProgress, ProgressSink};
use crate::source::{ArchiveCandidate, DataSource, MmapSource, OpenedSource, SourceKind};

/// How many decompressed bytes between extraction progress reports.
const PROGRESS_STEP: u64 = 1 << 20;

/// Safety limits for archives. These are deliberately generous but
/// bounded to prevent trivial zip-bombs from exhausting memory or disk.
pub const MAX_DECOMPRESSED_BYTES: u64 = 8 * 1024 * 1024 * 1024; // 8 GiB
pub const MAX_COMPRESSION_RATIO: u64 = 200;

/// Reason recorded for an entry that is password-protected. Compared by
/// value so callers can recognise it without parsing prose; the text is
/// also what the archive diagnostics panel shows the user.
pub const REJECTED_ENCRYPTED: &str = "encrypted (password-protected)";

#[derive(Debug)]
pub struct ArchiveSource {
    /// The original archive file.
    archive_path: PathBuf,
    /// All candidate entries (including rejected ones, for diagnostics).
    pub candidates: Vec<ArchiveCandidate>,
    /// In-memory listing of the archive for diagnostic purposes
    /// (kept around so we can re-extract entries on demand).
    archive_file: File,
}

impl ArchiveSource {
    pub fn from_path(path: &Path) -> Result<Self> {
        let archive_file = File::open(path)?;
        Self::new(path.to_path_buf(), archive_file)
    }

    fn new(archive_path: PathBuf, archive_file: File) -> Result<Self> {
        let mut zip = ZipArchive::new(archive_file.try_clone()?)?;
        let mut candidates = Vec::new();

        // Snapshot file names+lengths up front so we can release the
        // archive handle.
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index_raw(i).map(|f| f.name().to_string()))
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for (i, name) in names.iter().enumerate() {
            let raw = zip.by_index_raw(i)?;
            let compressed = raw.compressed_size();
            let uncompressed = raw.size();
            let (usable, rank, rejected_reason) = evaluate_entry(&raw, name);
            candidates.push(ArchiveCandidate {
                entry_name: name.clone(),
                compressed_size: compressed,
                uncompressed_size: uncompressed,
                usable,
                rank,
                rejected_reason,
            });
        }

        // Help caller localize mis-configured paths.
        for c in &candidates {
            if c.entry_name.contains("..") {
                return Err(AnvilError::PathTraversal(c.entry_name.clone()));
            }
        }

        Ok(Self {
            archive_path,
            candidates,
            archive_file,
        })
    }

    pub fn archive_name(&self) -> &str {
        self.archive_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
    }

    pub fn archive_path(&self) -> &Path {
        &self.archive_path
    }

    pub fn total_candidates(&self) -> usize {
        self.candidates.iter().filter(|c| c.usable).count()
    }

    /// The first entry that was skipped because it is encrypted, if any.
    /// Used by the loader to give a targeted message when an archive has
    /// data files but none of them can be read without a password.
    pub fn first_encrypted_entry(&self) -> Option<&ArchiveCandidate> {
        self.candidates
            .iter()
            .find(|c| !c.usable && c.rejected_reason == REJECTED_ENCRYPTED)
    }

    /// Resolves a single entry to an `OpenedSource` rooted at a temp file
    /// on disk. The caller does not own the temp file; it is cleaned up
    /// when the `OpenedSource` is dropped via the `MmapSource`'s reference
    /// to its underlying path.
    pub fn extract_into_temp(&self, entry_name: &str) -> Result<OpenedSource> {
        self.extract_into_temp_with_progress(entry_name, None)
    }

    /// [`extract_into_temp`](Self::extract_into_temp) that reports
    /// decompression progress. The entry's uncompressed size is known
    /// from the central directory, so the bar is determinate.
    pub fn extract_into_temp_with_progress(
        &self,
        entry_name: &str,
        progress: Option<ProgressSink<'_>>,
    ) -> Result<OpenedSource> {
        let mut zip = ZipArchive::new(self.archive_file.try_clone()?)?;
        let mut entry = zip
            .by_name(entry_name)
            .map_err(|_| AnvilError::EntryNotFound(entry_name.to_string()))?;

        if entry.encrypted() {
            return Err(AnvilError::EncryptedEntry(entry_name.to_string()));
        }
        if !is_supported_method(entry.compression()) {
            return Err(AnvilError::UnsupportedCompression(
                format!("{:?}", entry.compression()),
                entry_name.to_string(),
            ));
        }

        let uncompressed_size = entry.size();
        if uncompressed_size > MAX_DECOMPRESSED_BYTES {
            return Err(AnvilError::DecompressedTooLarge {
                name: entry_name.to_string(),
                actual: uncompressed_size,
                limit: MAX_DECOMPRESSED_BYTES,
            });
        }

        let dir = tempfile::tempdir()?;
        let safe_name = sanitize_filename(entry_name);
        let out_path = dir.path().join(&safe_name);
        let mut out = File::create(&out_path)?;
        let mut copied: u64 = 0;
        let mut buf = vec![0u8; 1 << 20]; // 1 MiB copy buffer
        let mut next_report: u64 = 0;

        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            if let Some(sink) = progress {
                if copied >= next_report {
                    sink(LoadProgress {
                        phase: LoadPhase::Extracting,
                        bytes_done: copied,
                        bytes_total: Some(uncompressed_size),
                    });
                    next_report = copied.saturating_add(PROGRESS_STEP);
                }
            }
            let c = entry.compressed_size();
            if c > 0 && copied > c.saturating_mul(MAX_COMPRESSION_RATIO) {
                return Err(AnvilError::DecompressedTooLarge {
                    name: entry_name.to_string(),
                    actual: copied,
                    limit: c.saturating_mul(MAX_COMPRESSION_RATIO),
                });
            }
            std::io::Write::write_all(&mut out, &buf[..n])?;
        }
        if let Some(sink) = progress {
            sink(LoadProgress {
                phase: LoadPhase::Extracting,
                bytes_done: copied,
                bytes_total: Some(uncompressed_size),
            });
        }
        out.sync_all()?;
        drop(out);

        let physical = MmapSource::open(&out_path)?;

        let candidate = self
            .candidates
            .iter()
            .find(|c| c.entry_name == *entry_name)
            .cloned()
            .ok_or_else(|| AnvilError::EntryNotFound(entry_name.to_string()))?;

        Ok(OpenedSource {
            source: DataSource {
                kind: SourceKind::ArchiveEntry {
                    archive_name: self.archive_name().to_string(),
                    entry_name: entry_name.to_string(),
                    compressed_size: candidate.compressed_size,
                    uncompressed_size: candidate.uncompressed_size,
                },
                path: self.archive_path.clone(),
            },
            physical: Arc::new(physical),
            archive_candidates: Some(self.candidates.clone()),
            temp_dir: Some(Arc::new(dir)),
        })
    }

    /// Returns the relative file name a candidate is stored under.
    /// If the archive has exactly one usable entry, this returns it.
    pub fn single_usable(&self) -> Option<&ArchiveCandidate> {
        let usable: Vec<&ArchiveCandidate> = self.candidates.iter().filter(|c| c.usable).collect();
        if usable.len() == 1 {
            Some(usable[0])
        } else {
            None
        }
    }
}

fn evaluate_entry(entry: &ZipFile<'_>, name: &str) -> (bool, u8, String) {
    if entry.is_dir() {
        return (false, u8::MAX, "directory".into());
    }
    // Cheap heuristic: __MACOSX, .DS_Store, hidden files in dot dirs.
    if name.contains("__MACOSX/") || name.contains("/__MACOSX/") {
        return (false, u8::MAX, "macOS metadata (__MACOSX)".into());
    }
    if name.ends_with("/.DS_Store") || name == ".DS_Store" {
        return (false, u8::MAX, "macOS metadata (.DS_Store)".into());
    }
    if name.starts_with('.') {
        return (false, u8::MAX, "hidden file".into());
    }
    // Reject path traversal outright.
    if name.contains("..") {
        return (false, u8::MAX, "suspicious path (contains '..')".into());
    }
    // Decompressed size guard.
    if entry.size() > MAX_DECOMPRESSED_BYTES {
        return (
            false,
            u8::MAX,
            format!("exceeds {} byte limit", MAX_DECOMPRESSED_BYTES),
        );
    }
    let Some(rank) = extension_rank(name) else {
        let ext = Path::new(name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        return (false, u8::MAX, format!("unsupported extension: .{ext}"));
    };
    // A plausible data file, but can we actually read it? Report the
    // reason up front so it shows in the archive diagnostics list rather
    // than only when the lazy extraction fails.
    if entry.encrypted() {
        return (false, u8::MAX, REJECTED_ENCRYPTED.into());
    }
    if !is_supported_method(entry.compression()) {
        return (
            false,
            u8::MAX,
            format!("unsupported compression ({:?})", entry.compression()),
        );
    }
    if ratio_exceeded(entry.compressed_size(), entry.size()) {
        return (
            false,
            u8::MAX,
            format!(
                "suspicious compression ratio ({}x > {}x limit)",
                compression_ratio(entry.compressed_size(), entry.size()),
                MAX_COMPRESSION_RATIO
            ),
        );
    }
    (true, rank, String::new())
}

/// Can the `zip` crate decompress this method for us?
fn is_supported_method(method: CompressionMethod) -> bool {
    matches!(
        method,
        CompressionMethod::Stored | CompressionMethod::Deflated | CompressionMethod::Bzip2
    )
}

/// Would reading this entry expand beyond the compression-ratio cap?
/// `compressed == 0` is treated as "no known limit" rather than an
/// infinite ratio, matching the streaming check in `extract_into_temp`.
fn ratio_exceeded(compressed: u64, uncompressed: u64) -> bool {
    compressed > 0 && uncompressed > compressed.saturating_mul(MAX_COMPRESSION_RATIO)
}

/// Uncompressed / compressed ratio, rounded down, for the reason text.
fn compression_ratio(compressed: u64, uncompressed: u64) -> u64 {
    uncompressed.checked_div(compressed).unwrap_or(0)
}

/// Rank a file by how likely it is to be the dataset the user wants.
/// Lower is better; `None` means unsupported.
fn extension_rank(name: &str) -> Option<u8> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "csv" | "tsv" | "tab" | "psv" => Some(0),
        "dat" | "out" => Some(1),
        "txt" => Some(2),
        _ => None,
    }
}

/// Make a safe filename for the temp directory; strips any directory
/// components so we never end up with `..` outside the temp dir.
fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == '\0' {
                '_'
            } else {
                c
            }
        })
        .collect();
    if cleaned.is_empty() {
        "entry.bin".to_string()
    } else {
        cleaned
    }
}

/// Read a small prefix of an archive entry and decide if it looks
/// delimiter-text. Used as the "I have no extension, what am I?" check.
pub fn sample_first_bytes(zip: &mut ZipArchive<File>, name: &str) -> Result<Vec<u8>> {
    let mut f = zip
        .by_name(name)
        .map_err(|_| AnvilError::EntryNotFound(name.to_string()))?;
    let mut buf = vec![0u8; 4096.min(f.size() as usize).max(64)];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_check_matches_the_streaming_extraction_limit() {
        // Exactly at the cap is allowed; one byte over is not.
        assert!(!ratio_exceeded(100, 100 * MAX_COMPRESSION_RATIO));
        assert!(ratio_exceeded(100, 100 * MAX_COMPRESSION_RATIO + 1));
        // Unknown compressed size never triggers the cap.
        assert!(!ratio_exceeded(0, u64::MAX));
        // A normal (near 1:1) entry is fine.
        assert!(!ratio_exceeded(1_000, 1_000));
    }

    #[test]
    fn compression_ratio_rounds_down_and_handles_zero() {
        assert_eq!(compression_ratio(100, 250), 2);
        assert_eq!(compression_ratio(0, 999), 0);
        assert_eq!(compression_ratio(7, 7), 1);
    }

    #[test]
    fn supported_methods_are_the_ones_we_can_decode() {
        assert!(is_supported_method(CompressionMethod::Stored));
        assert!(is_supported_method(CompressionMethod::Deflated));
        assert!(is_supported_method(CompressionMethod::Bzip2));
        assert!(!is_supported_method(CompressionMethod::SHRINK));
    }

    #[test]
    fn extension_rank_prefers_tabular_extensions() {
        assert_eq!(extension_rank("data.csv"), Some(0));
        assert_eq!(extension_rank("dir/DATA.TSV"), Some(0));
        assert_eq!(extension_rank("export.out"), Some(1));
        assert_eq!(extension_rank("notes.txt"), Some(2));
        assert_eq!(extension_rank("image.png"), None);
        assert_eq!(extension_rank("noext"), None);
    }
}

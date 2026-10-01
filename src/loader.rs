//! Open orchestration: turn a path into a browsable document.
//!
//! This is the logic the UI calls when a file is dropped or picked. It is
//! deliberately free of any UI dependency so it can be tested and driven
//! headlessly (see `anvil --probe`).

use std::path::Path;
use std::sync::Arc;

use crate::error::{AnvilError, Result};
use crate::parser::indexing::{self, IndexOptions};
use crate::parser::ParsedDocument;
use crate::progress::ProgressSink;
use crate::source::archive::ArchiveSource;
use crate::source::{open_path, OpenResult, OpenedSource};

/// A document ready to browse, plus where it came from.
pub struct LoadedDocument {
    pub document: ParsedDocument,
    /// Present when the document came from a ZIP archive.
    pub archive: Option<Arc<ArchiveSource>>,
    /// The archive entry that was selected, when applicable.
    pub entry: Option<String>,
}

/// Outcome of loading a path.
pub enum LoadResult {
    /// One document is ready to browse.
    Ready(Box<LoadedDocument>),
    /// A ZIP with several usable data files: the caller must choose one.
    NeedArchiveChoice(Arc<ArchiveSource>),
}

/// Index an already-opened source into a `ParsedDocument`.
pub fn index_opened(opened: OpenedSource) -> ParsedDocument {
    index_opened_with_progress(opened, None)
}

/// [`index_opened`] that reports indexing progress.
pub fn index_opened_with_progress(
    opened: OpenedSource,
    progress: Option<ProgressSink<'_>>,
) -> ParsedDocument {
    let result = indexing::parse_with_progress(&opened, IndexOptions::default(), progress);
    ParsedDocument {
        opened,
        meta: result.meta,
        record_offsets: result.record_offsets,
    }
}

/// Re-index an already-indexed document with different options, reusing
/// its memory map / extracted temp file. Used by the UI to apply a forced
/// header mode without re-opening the file or re-extracting an archive
/// entry. `ParsedDocument::opened` is shared by `Arc`, so the clone is
/// cheap and keeps the mmap alive.
pub fn reindex(doc: &ParsedDocument, options: IndexOptions) -> ParsedDocument {
    let result = indexing::parse_with(&doc.opened, options);
    ParsedDocument {
        opened: doc.opened.clone(),
        meta: result.meta,
        record_offsets: result.record_offsets,
    }
}

/// Open a path. Direct files resolve immediately; a ZIP with exactly one
/// usable entry opens transparently; a ZIP with several prompts the caller.
pub fn load(path: &Path) -> Result<LoadResult> {
    load_with_progress(path, None)
}

/// [`load`] that reports extraction and indexing progress.
pub fn load_with_progress(path: &Path, progress: Option<ProgressSink<'_>>) -> Result<LoadResult> {
    match open_path(path)? {
        OpenResult::Direct(opened) => Ok(LoadResult::Ready(Box::new(LoadedDocument {
            document: index_opened_with_progress(opened, progress),
            archive: None,
            entry: None,
        }))),
        OpenResult::Archive(source) => {
            let source = Arc::new(source);
            if source.total_candidates() == 0 {
                // A password-protected archive is the common case where the
                // extension looks like data but nothing is readable; say so
                // explicitly instead of the generic "no supported files".
                if let Some(entry) = source.first_encrypted_entry() {
                    return Err(AnvilError::EncryptedEntry(entry.entry_name.clone()));
                }
                return Err(AnvilError::NoUsableEntries);
            }
            // Auto-open when there is exactly one clear favourite. Two
            // equally-rankable files (e.g. two CSVs) require a choice.
            match best_candidate(&source) {
                Some(entry) => load_entry_with_progress(&source, &entry, progress)
                    .map(|doc| LoadResult::Ready(Box::new(doc))),
                None => Ok(LoadResult::NeedArchiveChoice(source)),
            }
        }
    }
}

/// Returns the entry to open automatically, or `None` when the archive has
/// several equally-preferred candidates and the user must choose.
fn best_candidate(source: &ArchiveSource) -> Option<String> {
    let mut usable: Vec<&crate::source::ArchiveCandidate> =
        source.candidates.iter().filter(|c| c.usable).collect();
    usable.sort_by_key(|c| c.rank);
    let best_rank = usable.first()?.rank;
    let mut best = usable.iter().filter(|c| c.rank == best_rank);
    let first = best.next()?;
    if best.next().is_some() {
        // More than one file shares the best rank: ambiguous.
        None
    } else {
        Some(first.entry_name.clone())
    }
}

/// Extract and index a specific entry from an open archive.
pub fn load_entry(source: &Arc<ArchiveSource>, entry: &str) -> Result<LoadedDocument> {
    load_entry_with_progress(source, entry, None)
}

/// [`load_entry`] that reports decompression and indexing progress.
pub fn load_entry_with_progress(
    source: &Arc<ArchiveSource>,
    entry: &str,
    progress: Option<ProgressSink<'_>>,
) -> Result<LoadedDocument> {
    let opened = source.extract_into_temp_with_progress(entry, progress)?;
    Ok(LoadedDocument {
        document: index_opened_with_progress(opened, progress),
        archive: Some(source.clone()),
        entry: Some(entry.to_string()),
    })
}

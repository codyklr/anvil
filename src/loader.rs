//! Open orchestration: turn a path into a browsable document.
//!
//! This is the logic the UI calls when a file is dropped or picked. It is
//! deliberately free of any UI dependency so it can be tested and driven
//! headlessly (see `anvil --probe`).

use std::path::Path;
use std::sync::Arc;

use crate::error::{AnvilError, Result};
use crate::parser::{indexing, ParsedDocument};
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
    let result = indexing::parse(&opened);
    ParsedDocument {
        opened,
        meta: result.meta,
        record_offsets: result.record_offsets,
    }
}

/// Open a path. Direct files resolve immediately; a ZIP with exactly one
/// usable entry opens transparently; a ZIP with several prompts the caller.
pub fn load(path: &Path) -> Result<LoadResult> {
    match open_path(path)? {
        OpenResult::Direct(opened) => Ok(LoadResult::Ready(Box::new(LoadedDocument {
            document: index_opened(opened),
            archive: None,
            entry: None,
        }))),
        OpenResult::Archive(source) => {
            let source = Arc::new(source);
            if source.total_candidates() == 0 {
                return Err(AnvilError::NoUsableEntries);
            }
            // Auto-open when there is exactly one clear favourite. Two
            // equally-rankable files (e.g. two CSVs) require a choice.
            match best_candidate(&source) {
                Some(entry) => {
                    load_entry(&source, &entry).map(|doc| LoadResult::Ready(Box::new(doc)))
                }
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
    let opened = source.extract_into_temp(entry)?;
    Ok(LoadedDocument {
        document: index_opened(opened),
        archive: Some(source.clone()),
        entry: Some(entry.to_string()),
    })
}

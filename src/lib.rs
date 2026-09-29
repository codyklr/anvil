//! Anvil core: data sources, parsing, indexing, archive handling.
//!
//! This crate has no GPUI dependency. The hover/designer UI lives in the
//! `anvil` binary module tree under `src/ui/`.

pub mod error;
pub mod loader;
pub mod parser;
pub mod query;
pub mod selection;
pub mod source;

pub use error::{AnvilError, Result};
pub use loader::{index_opened, load, load_entry, LoadResult, LoadedDocument};
pub use parser::indexing::{build as build_index, parse as parse_opened, IndexResult};
pub use parser::{Cell, Cells, ColumnIndex, ParseMeta, ParseWarning, ParsedDocument, WarningKind};
pub use query::{find_ranges, search as search_records, SearchOptions};
pub use selection::{CellPos, Selection};
pub use source::archive::{
    sample_first_bytes, ArchiveSource, MAX_COMPRESSION_RATIO, MAX_DECOMPRESSED_BYTES,
};
pub use source::{
    open_path, ArchiveCandidate, DataSource, MmapSource, OpenResult, OpenedSource, SourceKind,
};

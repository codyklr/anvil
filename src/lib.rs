//! Anvil core: data sources, parsing, indexing, archive handling.
//!
//! This crate has no GPUI dependency. The hover/designer UI lives in the
//! `anvil` binary module tree under `src/ui/`.

pub mod encoding;
pub mod error;
pub mod loader;
pub mod parser;
pub mod paths;
pub mod progress;
pub mod query;
pub mod schema;
pub mod selection;
pub mod source;

pub use encoding::{scan as scan_encoding, ColumnEncoding, EncodingReport};
pub use error::{AnvilError, Result};
pub use loader::{
    index_opened, index_opened_with_progress, load, load_entry, load_entry_with_progress,
    load_with_progress, reindex, LoadResult, LoadedDocument,
};
pub use parser::indexing::{
    build as build_index, build_with_progress as build_index_with_progress, parse as parse_opened,
    parse_with as parse_opened_with, parse_with_progress as parse_opened_with_progress,
    IndexOptions, IndexResult, DEFAULT_MAX_RECORD_BYTES,
};
pub use parser::{
    bom_len, column_label, exact_string, exact_string_capped, exact_string_window, strip_utf8_bom,
    Cell, Cells, ColumnIndex, ExactWindow, HeaderMode, ParseMeta, ParseWarning, ParsedDocument,
    WarningKind,
};
pub use paths::{anvil_data_dir, legacy_data_dir, model_root, PathsError};
pub use progress::{LoadPhase, LoadProgress, ProgressSink};
pub use query::filter::{
    apply as apply_filter, apply_expr as apply_filter_expr, parse as parse_filter,
    parse_expr as parse_filter_expr, Filter, FilterExpr, FilterOp,
};
pub use query::nl::{
    describe as describe_nl, interpret as interpret_nl, model_prompt, parse as parse_nl,
    strip_fences, validate_candidate as validate_nl_candidate, ModelBackend, ModelError, NlBackend,
    NlColumn, NlResult, NlSchema, RuleBackend,
};
pub use query::sort::{apply as sort_rows, SortKind, SortOrder, SortSpec};
pub use query::suggest::{
    candidates as suggest_candidates, context as suggest_context, Slot, SuggestContext,
    MAX_SUGGESTIONS,
};
pub use query::{find_ranges, search as search_records, SearchOptions};
pub use schema::stats::{
    sample_distinct, stats as column_stats, stats_rows as column_stats_rows, ColumnStats,
    DistinctCount, StatsOptions, ValueCount,
};
pub use schema::{infer as infer_schema, infer_column, ColumnProfile, ColumnType};
pub use selection::{CellPos, Selection};
pub use source::archive::{
    sample_first_bytes, ArchiveSource, MAX_COMPRESSION_RATIO, MAX_DECOMPRESSED_BYTES,
    REJECTED_ENCRYPTED,
};
pub use source::{
    open_path, ArchiveCandidate, DataSource, MmapSource, OpenResult, OpenedSource, SourceKind,
};
#[cfg(feature = "nl-gliner")]
pub mod gliner;

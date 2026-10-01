//! Build the byte-offset index for an `OpenedSource`.
//!
//! We walk the bytes, group physical lines that hold an unterminated
//! quote into one record, and record the start of each record into
//! `record_offsets`. The list has length `record_count + 1`; the final
//! entry is the length of the source.

use crate::parser::delimit::detect_with;
use crate::parser::{bom_len, HeaderMode, ParseMeta, ParseWarning, WarningKind};
use crate::progress::{LoadPhase, LoadProgress, ProgressSink};
use crate::source::OpenedSource;

/// How many bytes the indexer walks between progress reports. One
/// mebibyte keeps the callback frequency low enough not to matter for a
/// 650 MiB/s scan while still animating a determinate bar on a large
/// file.
const PROGRESS_STEP: usize = 1 << 20;

#[derive(Debug)]
pub struct IndexResult {
    pub meta: ParseMeta,
    pub record_offsets: Vec<u64>,
}

/// Default record-length limit for the `VeryLongRecord` diagnostic.
/// Chosen well above an ordinary CSV row: a multi-MB cell, or a record
/// that swallowed the rest of the file after an unterminated quote,
/// trips it while a merely large 1M-character cell does not.
pub const DEFAULT_MAX_RECORD_BYTES: u64 = 1 << 20; // 1 MiB

/// Indexing options. Currently only the record-length diagnostic limit;
/// it is threaded through so callers (and tests) can tune the threshold
/// without a process-wide constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexOptions {
    /// Records longer than this emit a `VeryLongRecord` warning. The
    /// length includes the record's trailing newline.
    pub max_record_bytes: u64,
    /// How the first row is interpreted. `Auto` runs the content
    /// heuristic; `Header` / `Headerless` force the interpretation.
    pub header_mode: HeaderMode,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            max_record_bytes: DEFAULT_MAX_RECORD_BYTES,
            header_mode: HeaderMode::Auto,
        }
    }
}

/// Build an index for `opened` given an initial `meta`. Mutates `meta` in
/// place: sets `record_count`, `header_range`, merges detect warnings,
/// and appends warnings encountered during walking. Uses the default
/// [`IndexOptions`].
pub fn build(opened: &OpenedSource, meta: ParseMeta) -> IndexResult {
    build_with(opened, meta, IndexOptions::default())
}

/// [`build`] with explicit options.
pub fn build_with(opened: &OpenedSource, meta: ParseMeta, options: IndexOptions) -> IndexResult {
    build_with_progress(opened, meta, options, None)
}

/// [`build_with`] that reports [`LoadProgress`] through `progress`.
pub fn build_with_progress(
    opened: &OpenedSource,
    meta: ParseMeta,
    options: IndexOptions,
    progress: Option<ProgressSink<'_>>,
) -> IndexResult {
    // Monomorphize a no-report version for the common case so the
    // per-byte progress check compiles away entirely. A runtime check
    // regressed the 1M-row scan from 254 ms to 622 ms.
    match progress {
        Some(sink) => build_impl::<true>(opened, meta, options, Some(sink)),
        None => build_impl::<false>(opened, meta, options, None),
    }
}

/// The indexing walk. `REPORT` is a compile-time switch: with `false`
/// the progress check is removed.
fn build_impl<const REPORT: bool>(
    opened: &OpenedSource,
    meta: ParseMeta,
    options: IndexOptions,
    progress: Option<ProgressSink<'_>>,
) -> IndexResult {
    let total = opened.len() as usize;
    let bytes = opened.bytes();
    // A leading UTF-8 BOM is structural, not data: skip it so the first
    // header/cell does not contain `EF BB BF`. The bytes stay in the
    // source and the encoding scan still reports the BOM.
    let bom = bom_len(bytes);

    let mut meta = meta;
    let mut warnings = std::mem::take(&mut meta.warnings);
    let delimiter = meta.delimiter;
    let quote = meta.quote;
    let has_headers = meta.has_headers;
    let expected_columns = if has_headers { meta.headers.len() } else { 0 };

    let n_cap = (total / 16).max(16);
    let mut offsets: Vec<u64> = Vec::with_capacity(n_cap.min(8 * 1024 * 1024));

    let mut pos: usize = bom;
    if has_headers {
        let newline_pos = match memchr::memchr(b'\n', &bytes[bom..]) {
            Some(p) => bom + p,
            None => total,
        };
        meta.header_range = Some((bom as u64, newline_pos as u64));
        pos = newline_pos + 1;
    }

    if pos < total {
        offsets.push(pos as u64);
    }

    // Progress reporting. `next_report` is only consulted when `REPORT`
    // is true; the const switch removes the check from the hot path.
    let mut next_report = usize::MAX;
    if REPORT {
        if let Some(sink) = progress {
            sink(LoadProgress {
                phase: LoadPhase::Indexing,
                bytes_done: pos as u64,
                bytes_total: Some(total as u64),
            });
            next_report = pos.saturating_add(PROGRESS_STEP);
        }
    }

    let mut in_quotes = false;
    // True whenever the cursor sits at the start of a field: at the start of
    // a record or immediately after a delimiter. A quote only opens a quoted
    // field from this position, so a quote mid-value is a literal character.
    let mut at_field_start = true;
    let mut cur_columns: usize = 0;
    let mut first_columns: Option<usize> = None;
    let mut max_columns: usize = expected_columns;
    let mut record_index: u64 = 0;
    // Whether any non-newline byte has been seen since the last boundary.
    // Used to count a final record that is not newline-terminated.
    let mut record_has_content = false;

    // NOTE: this byte-at-a-time loop measurably beats a `memchr3` skip
    // loop on dense CSVs (20 delimiters per row): the structural bytes
    // are too frequent for the SIMD scan to skip anything, while the
    // per-call scanner setup dominates. Kept simple on purpose. See the
    // 2026-11-10 performance-log entry.
    while pos < total {
        if REPORT && pos >= next_report {
            if let Some(sink) = progress {
                sink(LoadProgress {
                    phase: LoadPhase::Indexing,
                    bytes_done: pos as u64,
                    bytes_total: Some(total as u64),
                });
            }
            next_report = pos.saturating_add(PROGRESS_STEP);
        }
        let b = bytes[pos];

        if in_quotes {
            record_has_content = true;
            if b == quote {
                if pos + 1 < total && bytes[pos + 1] == quote {
                    pos += 2;
                    continue;
                }
                in_quotes = false;
            }
            pos += 1;
            continue;
        }

        if b == delimiter {
            cur_columns += 1;
            at_field_start = true;
            record_has_content = true;
        } else if b == quote && at_field_start {
            in_quotes = true;
            at_field_start = false;
            record_has_content = true;
        } else if b == b'\n' {
            cur_columns += 1;
            max_columns = max_columns.max(cur_columns);
            if expected_columns > 0 {
                if cur_columns != expected_columns {
                    warnings.push(ParseWarning {
                        kind: WarningKind::InconsistentColumnCount,
                        record_index: Some(record_index),
                        note: format!(
                            "header has {} columns, row has {cur_columns}",
                            expected_columns
                        ),
                    });
                }
            } else if let Some(first) = first_columns {
                if cur_columns != first {
                    warnings.push(ParseWarning {
                        kind: WarningKind::InconsistentColumnCount,
                        record_index: Some(record_index),
                        note: format!(
                            "row {record_index} has {cur_columns} cells, expected {first}"
                        ),
                    });
                }
            } else {
                first_columns = Some(cur_columns);
            }
            cur_columns = 0;
            at_field_start = true;
            record_has_content = false;
            record_index += 1;
            pos += 1;
            if pos < total {
                offsets.push(pos as u64);
            }
            continue;
        } else {
            at_field_start = false;
            record_has_content = true;
        }
        pos += 1;
    }

    if in_quotes {
        // `record_index` is the index of the partial record currently
        // being walked: it only advances when a newline closes a record,
        // and opening a quote never does. Subtracting one pointed at the
        // previous completed record whenever earlier rows existed, which
        // would send diagnostics navigation to the wrong row.
        warnings.push(ParseWarning {
            kind: WarningKind::UnterminatedQuote,
            record_index: Some(record_index),
            note: "file ended while inside a quoted field".into(),
        });
    }

    if record_has_content {
        max_columns = max_columns.max(cur_columns + 1);
        record_index += 1;
    }

    offsets.push(total as u64);

    // Flag any record that is unusually large. This is a post-pass over
    // the finished offset table so the walker above stays allocation-
    // free and single-minded. A record's byte length includes its
    // trailing newline (or the sentinel for the final record).
    for record in 0..record_index as usize {
        let (Some(&start), Some(&end)) = (offsets.get(record), offsets.get(record + 1)) else {
            break;
        };
        let len = end.saturating_sub(start);
        if len > options.max_record_bytes {
            warnings.push(ParseWarning {
                kind: WarningKind::VeryLongRecord,
                record_index: Some(record as u64),
                note: format!("record is {len} bytes (limit {})", options.max_record_bytes),
            });
        }
    }

    meta.record_count = record_index;
    meta.column_count = max_columns;
    meta.warnings = warnings;
    // In the no-report monomorphization `next_report` is never read; keep
    // the binding alive so the dead-code lint does not fire.
    let _ = next_report;
    if let Some(sink) = progress {
        sink(LoadProgress {
            phase: LoadPhase::Indexing,
            bytes_done: total as u64,
            bytes_total: Some(total as u64),
        });
    }
    IndexResult {
        meta,
        record_offsets: offsets,
    }
}

/// One-shot helper used by the binary: detect layout + index records.
pub fn parse(opened: &OpenedSource) -> IndexResult {
    parse_with(opened, IndexOptions::default())
}

/// [`parse`] with explicit options.
pub fn parse_with(opened: &OpenedSource, options: IndexOptions) -> IndexResult {
    parse_with_progress(opened, options, None)
}

/// [`parse_with`] that reports [`LoadProgress`] through `progress`.
pub fn parse_with_progress(
    opened: &OpenedSource,
    options: IndexOptions,
    progress: Option<ProgressSink<'_>>,
) -> IndexResult {
    let meta = detect_with(opened.bytes(), options.header_mode).into_meta();
    build_with_progress(opened, meta, options, progress)
}

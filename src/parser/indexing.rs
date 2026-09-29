//! Build the byte-offset index for an `OpenedSource`.
//!
//! We walk the bytes, group physical lines that hold an unterminated
//! quote into one record, and record the start of each record into
//! `record_offsets`. The list has length `record_count + 1`; the final
//! entry is the length of the source.

use crate::parser::delimit::detect;
use crate::parser::{ParseMeta, ParseWarning, WarningKind};
use crate::source::OpenedSource;

#[derive(Debug)]
pub struct IndexResult {
    pub meta: ParseMeta,
    pub record_offsets: Vec<u64>,
}

/// Build an index for `opened` given an initial `meta`. Mutates `meta` in
/// place: sets `record_count`, `header_range`, merges detect warnings,
/// and appends warnings encountered during walking.
pub fn build(opened: &OpenedSource, meta: ParseMeta) -> IndexResult {
    let total = opened.len() as usize;
    let bytes = opened.bytes();

    let mut meta = meta;
    let mut warnings = std::mem::take(&mut meta.warnings);
    let delimiter = meta.delimiter;
    let quote = meta.quote;
    let has_headers = meta.has_headers;
    let expected_columns = if has_headers { meta.headers.len() } else { 0 };

    let n_cap = (total / 16).max(16);
    let mut offsets: Vec<u64> = Vec::with_capacity(n_cap.min(8 * 1024 * 1024));

    let mut pos: usize = 0;
    if has_headers {
        let newline_pos = match memchr::memchr(b'\n', bytes) {
            Some(p) => p,
            None => total,
        };
        meta.header_range = Some((0, newline_pos as u64));
        pos = newline_pos + 1;
    }

    if pos < total {
        offsets.push(pos as u64);
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

    while pos < total {
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
        warnings.push(ParseWarning {
            kind: WarningKind::UnterminatedQuote,
            record_index: Some(record_index.saturating_sub(1)),
            note: "file ended while inside a quoted field".into(),
        });
    }

    if record_has_content {
        max_columns = max_columns.max(cur_columns + 1);
        record_index += 1;
    }

    offsets.push(total as u64);
    meta.record_count = record_index;
    meta.column_count = max_columns;
    meta.warnings = warnings;
    IndexResult {
        meta,
        record_offsets: offsets,
    }
}

/// One-shot helper used by the binary: detect layout + index records.
pub fn parse(opened: &OpenedSource) -> IndexResult {
    let meta = detect(opened.bytes()).into_meta();
    build(opened, meta)
}

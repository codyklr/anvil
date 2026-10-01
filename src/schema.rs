//! Heuristic per-column type inference.
//!
//! The column inspector and the sorter both need a cheap answer to
//! "is this column numeric or text?". This module scans a bounded
//! sample of each column once and assigns one of a few broad types.
//! It is deliberately heuristic: inference reports how much of the
//! sample looked numeric so callers can treat borderline columns as
//! text. It is GPUI-free and lives in `anvil_core`.
//!
//! [`stats`] builds on this to compute the column inspector's metrics.

pub mod stats;

use crate::parser::{ColumnIndex, ParsedDocument};

/// Broad inferred type for a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// Every non-empty sampled value parsed as an integer.
    Integer,
    /// At least [`NUMERIC_THRESHOLD`] of non-empty sampled values parsed
    /// as a number (integer or float).
    Float,
    /// Every non-empty sampled value was `true` or `false`
    /// (case-insensitive).
    Boolean,
    /// None of the above.
    Text,
    /// Every sampled value was empty.
    Empty,
}

impl ColumnType {
    /// Is this a numeric type (so the sorter should compare numbers)?
    pub fn is_numeric(self) -> bool {
        matches!(self, ColumnType::Integer | ColumnType::Float)
    }
}

/// The result of inferring one column's type.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnProfile {
    /// Header name, or `col0` for headerless columns.
    pub name: String,
    /// The inferred broad type.
    pub column_type: ColumnType,
    /// How many records were sampled.
    pub sampled_rows: u64,
    /// How many sampled values were non-empty.
    pub non_empty: u64,
    /// Fraction of non-empty sampled values that parsed as a number
    /// (`0.0` when there were none). This is the confidence signal.
    pub numeric_fraction: f32,
}

/// How many records to sample per column. Bounds inference cost so it
/// stays cheap on million-row files while still being representative.
pub const INFER_SAMPLE_ROWS: u64 = 2048;

/// Fraction of non-empty sampled values that must parse numerically for
/// a column to be treated as [`ColumnType::Float`].
pub const NUMERIC_THRESHOLD: f32 = 0.9;

/// Infer a type for every column of `doc`.
pub fn infer(doc: &ParsedDocument) -> Vec<ColumnProfile> {
    let column_count = doc.column_count();
    let sampled_rows = doc.record_count().min(INFER_SAMPLE_ROWS);
    (0..column_count)
        .map(|column| profile_from_rows(doc, column, sampled_rows, 0..sampled_rows))
        .collect()
}

/// Infer the type of a single column. Same sampling as [`infer`].
pub fn infer_column(doc: &ParsedDocument, column: usize) -> ColumnProfile {
    let sampled_rows = doc.record_count().min(INFER_SAMPLE_ROWS);
    profile_from_rows(doc, column, sampled_rows, 0..sampled_rows)
}

/// Infer the type of a single column from `rows` only. `rows` are record
/// indexes in display order (a filtered/sorted view); at most
/// [`INFER_SAMPLE_ROWS`] of them are sampled, so the inferred type
/// matches the row set [`crate::schema::stats::stats_rows`] measures.
pub fn infer_column_rows(doc: &ParsedDocument, column: usize, rows: &[u64]) -> ColumnProfile {
    let sampled = rows.len().min(INFER_SAMPLE_ROWS as usize);
    profile_from_rows(doc, column, sampled as u64, rows[..sampled].iter().copied())
}

fn profile_from_rows<I>(
    doc: &ParsedDocument,
    column: usize,
    sampled_rows: u64,
    rows: I,
) -> ColumnProfile
where
    I: Iterator<Item = u64>,
{
    let mut non_empty = 0u64;
    let mut integers = 0u64;
    let mut numbers = 0u64;
    let mut booleans = 0u64;

    for row in rows {
        let Some(cell) = doc.cell(row, ColumnIndex(column)) else {
            continue;
        };
        let trimmed = cell.as_str().trim();
        if trimmed.is_empty() {
            continue;
        }
        non_empty += 1;
        if trimmed.parse::<i64>().is_ok() {
            integers += 1;
        }
        if trimmed.parse::<f64>().is_ok() {
            numbers += 1;
        }
        if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("false") {
            booleans += 1;
        }
    }

    let numeric_fraction = if non_empty == 0 {
        0.0
    } else {
        numbers as f32 / non_empty as f32
    };

    let column_type = if non_empty == 0 {
        ColumnType::Empty
    } else if integers == non_empty {
        ColumnType::Integer
    } else if numeric_fraction >= NUMERIC_THRESHOLD {
        ColumnType::Float
    } else if booleans == non_empty {
        ColumnType::Boolean
    } else {
        ColumnType::Text
    };

    let name = crate::parser::column_label(&doc.meta.headers, column);

    ColumnProfile {
        name,
        column_type,
        sampled_rows,
        non_empty,
        numeric_fraction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_from(bytes: &[u8]) -> ParsedDocument {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("infer.csv");
        std::fs::write(&path, bytes).expect("write");
        let opened = crate::source::local::open_local(&path).expect("open");
        crate::index_opened(opened)
    }

    #[test]
    fn infers_integers() {
        let doc = doc_from(b"num,pad\n1,a\n2,a\n-3,a\n");
        let profiles = infer(&doc);
        assert_eq!(profiles[0].column_type, ColumnType::Integer);
        assert!(profiles[0].column_type.is_numeric());
        assert_eq!(profiles[0].non_empty, 3);
        assert_eq!(profiles[0].numeric_fraction, 1.0);
    }

    #[test]
    fn infers_floats_when_mixed_with_integers() {
        let doc = doc_from(b"amount,pad\n1.5,a\n2,a\n3.25,a\n");
        let profiles = infer(&doc);
        assert_eq!(profiles[0].column_type, ColumnType::Float);
    }

    #[test]
    fn infers_booleans() {
        let doc = doc_from(b"ok,pad\ntrue,a\nfalse,a\nTRUE,a\n");
        let profiles = infer(&doc);
        assert_eq!(profiles[0].column_type, ColumnType::Boolean);
        assert!(!profiles[0].column_type.is_numeric());
    }

    #[test]
    fn infers_text_for_mixed_values() {
        let doc = doc_from(b"country,pad\nUS,a\nUK,a\nDE,a\n");
        let profiles = infer(&doc);
        assert_eq!(profiles[0].column_type, ColumnType::Text);
    }

    #[test]
    fn infers_empty_for_all_empty() {
        let doc = doc_from(b"a,b\n,1\n,2\n");
        let profiles = infer(&doc);
        assert_eq!(profiles[0].column_type, ColumnType::Empty);
        assert_eq!(profiles[0].non_empty, 0);
        assert_eq!(profiles[0].numeric_fraction, 0.0);
    }

    #[test]
    fn a_few_non_numeric_values_still_read_as_numeric() {
        // 9 numbers + 1 sentinel = 0.9 numeric, at the threshold.
        let doc = doc_from(b"v,pad\n1,a\n2,a\n3,a\n4,a\n5,a\n6,a\n7,a\n8,a\n9,a\nn/a,a\n");
        let profiles = infer(&doc);
        assert_eq!(profiles[0].column_type, ColumnType::Float);
    }

    #[test]
    fn many_non_numeric_values_read_as_text() {
        let doc = doc_from(b"v,pad\n1,a\n2,a\n3,a\n4,a\n5,a\n6,a\n7,a\nn/a,a\nn/a,a\nn/a,a\n");
        let profiles = infer(&doc);
        assert_eq!(profiles[0].column_type, ColumnType::Text);
    }

    #[test]
    fn header_names_and_fallback() {
        let doc = doc_from(b"id,name\n1,a\n");
        let profiles = infer(&doc);
        assert_eq!(profiles[0].name, "id");
        assert_eq!(profiles[1].name, "name");
    }

    #[test]
    fn all_empty_column_has_no_profiles_of_interest() {
        let doc = doc_from(b"a,b\n,\n");
        let profiles = infer(&doc);
        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().all(|p| p.column_type == ColumnType::Empty));
    }
}

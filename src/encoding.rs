//! UTF-8 validity diagnostics.
//!
//! `Cell::as_str` and `into_string` silently fall back to empty/lossy
//! output for bytes that are not valid UTF-8, so an invalid cell renders
//! as if it were empty. This module scans a document once and reports
//! which columns contain such cells, plus whether the file begins with a
//! UTF-8 BOM.
//!
//! The scan is O(file length) and is meant to run on the background
//! executor (like `schema::stats`), not during open. It does not
//! allocate per cell: each record is visited with the same `Cells`
//! iterator the grid uses, and only a per-column counter is retained.

use crate::parser::{Cells, ParsedDocument};

/// One column with at least one non-UTF-8 cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnEncoding {
    pub column: usize,
    /// Header name when present and non-empty, otherwise the generated
    /// `col0`/`col1` label.
    pub name: String,
    /// Number of cells in this column whose decoded bytes are not valid
    /// UTF-8.
    pub invalid_cells: u64,
}

/// Result of scanning a document for encoding problems.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EncodingReport {
    /// Whether the file begins with the UTF-8 BOM (`EF BB BF`). The
    /// parser does not strip it, so it would otherwise end up glued to
    /// the first header cell.
    pub bom: bool,
    /// Data records visited by the scan (excludes the header row).
    pub records_scanned: u64,
    /// Columns with invalid cells, in column order. Empty when every
    /// scanned cell is valid UTF-8.
    pub columns: Vec<ColumnEncoding>,
}

impl EncodingReport {
    pub fn has_invalid_utf8(&self) -> bool {
        !self.columns.is_empty()
    }

    /// Total invalid cells across every column.
    pub fn total_invalid_cells(&self) -> u64 {
        self.columns.iter().map(|c| c.invalid_cells).sum()
    }
}

/// Scan every data record and count non-UTF-8 cells per column.
///
/// Cells beyond a record's actual width are not counted (a short row is
/// an inconsistent-width diagnostic, not an encoding one). Columns with
/// zero invalid cells are omitted from the report.
pub fn scan(doc: &ParsedDocument) -> EncodingReport {
    let bytes = doc.opened.bytes();
    let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let delimiter = doc.meta.delimiter;
    let quote = doc.meta.quote;
    let mut invalid = vec![0u64; doc.column_count()];

    for record in 0..doc.record_count() {
        let Some(record_bytes) = doc.record_bytes(record) else {
            continue;
        };
        for (column, cell) in Cells::new(record_bytes, delimiter, quote).enumerate() {
            let Some(counter) = invalid.get_mut(column) else {
                break;
            };
            if !cell.is_empty() && std::str::from_utf8(cell.as_ref()).is_err() {
                *counter += 1;
            }
        }
    }

    let columns = invalid
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .map(|(column, count)| ColumnEncoding {
            column,
            name: crate::parser::column_label(&doc.meta.headers, column),
            invalid_cells: *count,
        })
        .collect();

    EncodingReport {
        bom,
        records_scanned: doc.record_count(),
        columns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::local::open_local;

    fn parse(bytes: &[u8]) -> (ParsedDocument, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("encoding.csv");
        std::fs::write(&path, bytes).expect("write fixture");
        let opened = open_local(&path).expect("open local");
        let result = crate::parser::indexing::parse(&opened);
        let doc = ParsedDocument {
            opened,
            meta: result.meta,
            record_offsets: result.record_offsets,
        };
        (doc, dir)
    }

    #[test]
    fn clean_utf8_has_no_columns() {
        let (doc, _dir) = parse("a,b\n1,2\n3,\u{00e9}\n".as_bytes());
        let report = scan(&doc);
        assert!(!report.bom);
        assert!(!report.has_invalid_utf8());
        assert_eq!(report.total_invalid_cells(), 0);
        assert_eq!(report.records_scanned, 2);
    }

    #[test]
    fn invalid_utf8_is_counted_per_column() {
        let (doc, _dir) = parse(b"a,b\n1,\xff\n2,\xfe\xfe\n3,ok\n");
        let report = scan(&doc);
        assert!(report.has_invalid_utf8());
        assert_eq!(report.total_invalid_cells(), 2);
        assert_eq!(report.columns.len(), 1);
        assert_eq!(report.columns[0].column, 1);
        assert_eq!(report.columns[0].name, "b");
        assert_eq!(report.columns[0].invalid_cells, 2);
    }

    #[test]
    fn invalid_utf8_in_first_column_is_reported_by_name_fallback() {
        let (doc, _dir) = parse(b"\xff,b\n1,2\n");
        let report = scan(&doc);
        // The invalid byte is in the header, which the scan does not
        // visit; only data records count. Name stays the (lossy) header.
        assert_eq!(report.columns.len(), 0);

        let (doc, _dir) = parse(b"1,2\n\xff,3\n");
        let report = scan(&doc);
        assert_eq!(report.columns[0].column, 0);
        assert_eq!(report.columns[0].name, "col0");
        assert_eq!(report.columns[0].invalid_cells, 1);
    }

    #[test]
    fn utf8_bom_is_detected() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"a,b\n1,2\n");
        let (doc, _dir) = parse(&bytes);
        assert!(scan(&doc).bom);
    }
}

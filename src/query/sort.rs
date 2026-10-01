//! Column sorting over a `ParsedDocument`.
//!
//! Sorting produces a permutation of record indexes rather than copies
//! of the rows, matching the filter/search contract: the grid maps each
//! display position through the permutation to a record index. The
//! module is GPUI-free and testable in isolation.
//!
//! Empty values always sort to the end, for both ascending and
//! descending order, so blank cells do not scatter through the result.
//! Numeric columns compare numerically; everything else compares
//! case-insensitively on trimmed bytes (locale-free). The sort is
//! stable, so equal keys keep their previous order.

use std::cmp::Ordering;

use crate::parser::{ColumnIndex, ParsedDocument};
use crate::schema::ColumnType;

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// How values are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKind {
    /// Parse values as `f64`; non-numeric and empty values sort last.
    Numeric,
    /// Case-insensitive byte comparison of the trimmed value.
    Text,
}

impl SortKind {
    /// Pick the comparison for an inferred column type.
    pub fn for_column_type(column_type: ColumnType) -> Self {
        if column_type.is_numeric() {
            SortKind::Numeric
        } else {
            SortKind::Text
        }
    }
}

/// A sort request: which column, which direction, and how to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub column: usize,
    pub order: SortOrder,
    pub kind: SortKind,
}

/// Sort `rows` (record indexes) by `spec`, returning a new `Vec` in the
/// requested order. `rows` is the current view - usually the filtered
/// set, or every record when there is no filter - so sorting composes
/// with filtering.
pub fn apply(doc: &ParsedDocument, rows: &[u64], spec: &SortSpec) -> Vec<u64> {
    // Decode each value once into a comparable key. `sort_by_cached_key`
    // would also work, but an explicit key vector lets empty-last apply
    // to both directions without reversing the empty ordering.
    let keys: Vec<Key> = rows.iter().map(|&row| Key::new(doc, row, spec)).collect();
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| compare(&keys[a], &keys[b], spec.order));
    order.into_iter().map(|index| rows[index]).collect()
}

enum Key {
    /// Empty or (for a numeric sort) non-numeric: always sorted last.
    Missing,
    Number(f64),
    Text(Vec<u8>),
}

impl Key {
    fn new(doc: &ParsedDocument, row: u64, spec: &SortSpec) -> Self {
        let Some(cell) = doc.cell(row, ColumnIndex(spec.column)) else {
            return Key::Missing;
        };
        let trimmed = cell.as_str().trim();
        if trimmed.is_empty() {
            return Key::Missing;
        }
        match spec.kind {
            SortKind::Numeric => trimmed
                .parse::<f64>()
                .map(Key::Number)
                .unwrap_or(Key::Missing),
            SortKind::Text => {
                let mut bytes: Vec<u8> = trimmed.bytes().map(|b| b.to_ascii_lowercase()).collect();
                bytes.shrink_to_fit();
                Key::Text(bytes)
            }
        }
    }
}

fn compare(a: &Key, b: &Key, order: SortOrder) -> Ordering {
    match (a, b) {
        (Key::Missing, Key::Missing) => Ordering::Equal,
        // Empty / non-numeric always last, regardless of direction.
        (Key::Missing, _) => Ordering::Greater,
        (_, Key::Missing) => Ordering::Less,
        _ => {
            let base = match (a, b) {
                (Key::Number(x), Key::Number(y)) => x.total_cmp(y),
                (Key::Text(x), Key::Text(y)) => x.cmp(y),
                _ => Ordering::Equal,
            };
            match order {
                SortOrder::Ascending => base,
                SortOrder::Descending => base.reverse(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::infer;

    fn doc_from(bytes: &[u8]) -> ParsedDocument {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sort.csv");
        std::fs::write(&path, bytes).expect("write");
        let opened = crate::source::local::open_local(&path).expect("open");
        crate::index_opened(opened)
    }

    fn spec(doc: &ParsedDocument, column: usize, order: SortOrder) -> SortSpec {
        let profile = infer(doc);
        SortSpec {
            column,
            order,
            kind: SortKind::for_column_type(profile[column].column_type),
        }
    }

    fn all_rows(doc: &ParsedDocument) -> Vec<u64> {
        (0..doc.record_count()).collect()
    }

    #[test]
    fn sorts_integers_ascending_numerically() {
        let doc = doc_from(b"num,pad\n10,a\n2,a\n1,a\n");
        let rows = all_rows(&doc);
        let sorted = apply(&doc, &rows, &spec(&doc, 0, SortOrder::Ascending));
        // Record indexes: 0->10, 1->2, 2->1. Numeric asc = 2,1,0.
        assert_eq!(sorted, vec![2, 1, 0]);
    }

    #[test]
    fn sorts_integers_descending() {
        let doc = doc_from(b"num,pad\n10,a\n2,a\n1,a\n");
        let rows = all_rows(&doc);
        let sorted = apply(&doc, &rows, &spec(&doc, 0, SortOrder::Descending));
        assert_eq!(sorted, vec![0, 1, 2]);
    }

    #[test]
    fn numeric_sort_does_not_compare_lexicographically() {
        // Lexicographic would put "100" before "20" before "3".
        let doc = doc_from(b"num,pad\n100,a\n20,a\n3,a\n");
        let rows = all_rows(&doc);
        let sorted = apply(&doc, &rows, &spec(&doc, 0, SortOrder::Ascending));
        assert_eq!(sorted, vec![2, 1, 0]);
    }

    #[test]
    fn sorts_text_case_insensitively() {
        let doc = doc_from(b"country,pad\nbanana,a\nApple,a\ncherry,a\n");
        let rows = all_rows(&doc);
        let sorted = apply(&doc, &rows, &spec(&doc, 0, SortOrder::Ascending));
        assert_eq!(sorted, vec![1, 0, 2]);
    }

    #[test]
    fn empty_values_sort_to_the_end_both_directions() {
        let doc = doc_from(b"v,pad\n3,a\n,a\n1,a\n");
        let rows = all_rows(&doc);
        let asc = apply(&doc, &rows, &spec(&doc, 0, SortOrder::Ascending));
        assert_eq!(asc, vec![2, 0, 1]);
        let desc = apply(&doc, &rows, &spec(&doc, 0, SortOrder::Descending));
        assert_eq!(desc, vec![0, 2, 1]);
    }

    #[test]
    fn non_numeric_values_sort_last_in_a_numeric_column() {
        let doc = doc_from(b"v,pad\n5,a\nn/a,a\n1,a\n2,a\n");
        let rows = all_rows(&doc);
        // Force a numeric sort even though the mixed column infers as text.
        let spec = SortSpec {
            column: 0,
            order: SortOrder::Ascending,
            kind: SortKind::Numeric,
        };
        let sorted = apply(&doc, &rows, &spec);
        assert_eq!(sorted, vec![2, 3, 0, 1]);
    }

    #[test]
    fn sort_composes_with_a_filter_subset() {
        let doc = doc_from(b"country,amount\nUS,3\nUK,1\nUS,2\n");
        let spec = spec(&doc, 1, SortOrder::Ascending);
        // Pretend the filter kept only the US rows (0 and 2).
        let filtered = vec![0u64, 2];
        let sorted = apply(&doc, &filtered, &spec);
        assert_eq!(sorted, vec![2, 0]);
    }

    #[test]
    fn sort_is_stable_for_equal_keys() {
        let doc = doc_from(b"k,v\nx,1\nx,2\ny,3\n");
        let rows = all_rows(&doc);
        let sorted = apply(&doc, &rows, &spec(&doc, 0, SortOrder::Ascending));
        // The two "x" rows keep their original relative order.
        assert_eq!(sorted, vec![0, 1, 2]);
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let doc = doc_from(b"num,pad\n1,a\n");
        let sorted = apply(&doc, &[], &spec(&doc, 0, SortOrder::Ascending));
        assert!(sorted.is_empty());
    }
}

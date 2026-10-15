//! Column sorting over a `ParsedDocument`.
//!
//! Sorting produces a permutation of record indexes rather than copies
//! of the rows, matching the filter/search contract: the grid maps each
//! display position through the permutation to a record index. The
//! module is GPUI-free and testable in isolation.
//!
//! A zero-byte cell sorts last, in both directions, so a blank does not
//! scatter through the result. A whitespace-only cell is a value.
//! Numeric columns compare numerically; everything else compares exact
//! bytes (case-sensitive, padding preserved) so `Apple` and `apple`, or
//! ` 42 ` and `42`, never compare equal. Short text keys are copied into
//! one buffer so comparisons stay close together. A longer cell is
//! borrowed from the mapped file, so a huge value is not duplicated for
//! the sort. Ties fall back to the record index for a deterministic
//! order.

use std::borrow::Cow;
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
    /// Exact byte comparison (case-sensitive, padding preserved).
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
    // Short text values share one buffer. Pointing every key at the
    // mapped file was slower: a million comparisons then miss cache
    // across the whole CSV. A cell above `PACK_LIMIT` stays borrowed
    // so a multi-megabyte value is not copied.
    let mut packed = Vec::new();
    if spec.kind == SortKind::Text {
        packed.reserve(rows.len().saturating_mul(16));
    }
    let keys: Vec<Key> = rows
        .iter()
        .map(|&row| Key::new(doc, row, spec, &mut packed))
        .collect();
    let mut order: Vec<usize> = (0..rows.len()).collect();
    // Tie-break equal keys on the record index so the result is a total,
    // deterministic order even if the input `rows` order ever changes.
    order.sort_by(|&a, &b| {
        compare(&keys[a], &keys[b], &packed, spec.order)
            .then_with(|| {
                // Equal numbers keep the file's spelling. "007" and "7"
                // are the same f64, and the bytes are what still differs.
                if matches!((&keys[a], &keys[b]), (Key::Number(_), Key::Number(_))) {
                    cell_bytes(doc, rows[a], spec.column).cmp(&cell_bytes(
                        doc,
                        rows[b],
                        spec.column,
                    ))
                } else {
                    Ordering::Equal
                }
            })
            .then_with(|| rows[a].cmp(&rows[b]))
    });
    order.into_iter().map(|index| rows[index]).collect()
}

/// Text at least this long is compared in place. Shorter values are
/// packed so the sort does not chase one allocation, or one record,
/// per cell.
const PACK_LIMIT: usize = 128;

enum Key<'a> {
    /// Empty or (for a numeric sort) non-numeric: always sorted last.
    Missing,
    Number(f64),
    /// `packed[start..end]`.
    Packed {
        start: usize,
        end: usize,
    },
    /// A long cell, still sitting in the mapped file or in an unescape
    /// buffer that the key owns.
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
}

impl<'a> Key<'a> {
    fn new(doc: &'a ParsedDocument, row: u64, spec: &SortSpec, packed: &mut Vec<u8>) -> Self {
        let Some(cell) = doc.cell(row, ColumnIndex(spec.column)) else {
            return Key::Missing;
        };
        // Only a zero-byte cell is missing. A whitespace-only cell is a
        // value (ADR 0003): text sorts it by those bytes, and a numeric
        // sort rejects it at the parse below.
        if cell.as_bytes().is_empty() {
            return Key::Missing;
        }
        match spec.kind {
            SortKind::Numeric => {
                let trimmed = cell.as_str().trim();
                if trimmed.is_empty() {
                    return Key::Missing;
                }
                trimmed
                    .parse::<f64>()
                    .map(Key::Number)
                    .unwrap_or(Key::Missing)
            }
            SortKind::Text => pack_text(cell.into_bytes(), packed),
        }
    }
}

fn pack_text<'a>(bytes: Cow<'a, [u8]>, packed: &mut Vec<u8>) -> Key<'a> {
    if bytes.len() > PACK_LIMIT {
        return match bytes {
            Cow::Borrowed(bytes) => Key::Borrowed(bytes),
            Cow::Owned(bytes) => Key::Owned(bytes),
        };
    }
    let start = packed.len();
    packed.extend_from_slice(&bytes);
    Key::Packed {
        start,
        end: packed.len(),
    }
}

/// Decoded bytes for a numeric tiebreak. Borrowed unless the cell
/// itself had to unescape a quote.
fn cell_bytes(doc: &ParsedDocument, row: u64, column: usize) -> Cow<'_, [u8]> {
    match doc.cell(row, ColumnIndex(column)) {
        Some(cell) => cell.into_bytes(),
        None => Cow::Borrowed(b""),
    }
}

fn text_of<'a>(key: &'a Key<'_>, packed: &'a [u8]) -> &'a [u8] {
    match key {
        Key::Packed { start, end } => &packed[*start..*end],
        Key::Borrowed(bytes) => bytes,
        Key::Owned(bytes) => bytes,
        Key::Missing | Key::Number(_) => b"",
    }
}

fn compare(a: &Key, b: &Key, packed: &[u8], order: SortOrder) -> Ordering {
    match (a, b) {
        (Key::Missing, Key::Missing) => Ordering::Equal,
        // Empty / non-numeric always last, regardless of direction.
        (Key::Missing, _) => Ordering::Greater,
        (_, Key::Missing) => Ordering::Less,
        _ => {
            let base = match (a, b) {
                (Key::Number(x), Key::Number(y)) => x.total_cmp(y),
                (
                    Key::Packed { .. } | Key::Borrowed(_) | Key::Owned(_),
                    Key::Packed { .. } | Key::Borrowed(_) | Key::Owned(_),
                ) => text_of(a, packed).cmp(text_of(b, packed)),
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
    fn sorts_text_by_exact_bytes() {
        let doc = doc_from(b"country,pad\nbanana,a\nApple,a\ncherry,a\n");
        let rows = all_rows(&doc);
        let sorted = apply(&doc, &rows, &spec(&doc, 0, SortOrder::Ascending));
        // Exact bytes: Apple (0x41) < banana (0x62) < cherry (0x63).
        assert_eq!(sorted, vec![1, 0, 2]);
    }

    #[test]
    fn escaped_quotes_sort_by_the_unescaped_bytes() {
        // The quoted cell is `a"b` after unescaping. That must sort as
        // the decoded bytes, including when the key owns the unescape.
        // One column of text, so the first line is a record too:
        // "v", a"b, "a", "b".
        let doc = doc_from(b"v\n\"a\"\"b\"\na\nb\n");
        let rows = all_rows(&doc);
        let spec = spec(&doc, 0, SortOrder::Ascending);
        assert_eq!(spec.kind, SortKind::Text);
        assert_eq!(
            doc.cell(1, ColumnIndex(0)).expect("quoted cell").as_bytes(),
            b"a\"b"
        );
        // "a" < "a\"b" < "b" < "v".
        assert_eq!(apply(&doc, &rows, &spec), vec![2, 1, 3, 0]);
    }

    #[test]
    fn long_text_sorts_with_the_short_values() {
        // Longer than `PACK_LIMIT`, so this cell is compared from the
        // mapped file rather than the packed buffer.
        let long = format!("m{}", "x".repeat(200));
        let csv = format!("v,pad\n{long},a\na,a\nz,a\n");
        let doc = doc_from(csv.as_bytes());
        let rows = all_rows(&doc);
        let spec = spec(&doc, 0, SortOrder::Ascending);
        assert_eq!(spec.kind, SortKind::Text);
        assert!(
            doc.cell(0, ColumnIndex(0))
                .expect("long cell")
                .as_bytes()
                .len()
                > 128
        );
        // "a" < "mxxxx..." < "z".
        assert_eq!(apply(&doc, &rows, &spec), vec![1, 0, 2]);
    }

    #[test]
    fn text_sort_distinguishes_case_and_padding() {
        let doc = doc_from(b"v,pad\napple,a\nApple,a\n");
        let rows = all_rows(&doc);
        // Uppercase sorts before lowercase by exact bytes; the old
        // case-insensitive key would have treated these as equal.
        assert_eq!(
            apply(&doc, &rows, &spec(&doc, 0, SortOrder::Ascending)),
            vec![1, 0]
        );
        assert_eq!(
            apply(&doc, &rows, &spec(&doc, 0, SortOrder::Descending)),
            vec![0, 1]
        );

        let padded = doc_from(b"v,pad\n a,a\na,a\n");
        let rows = all_rows(&padded);
        // Leading whitespace is part of the key: " a" (0x20) < "a".
        assert_eq!(
            apply(&padded, &rows, &spec(&padded, 0, SortOrder::Ascending)),
            vec![0, 1]
        );
    }

    #[test]
    fn whitespace_only_text_sorts_by_exact_bytes() {
        let doc = doc_from(b"v,pad\n,a\n\" \",a\na,a\n");
        let rows = all_rows(&doc);
        let spec = spec(&doc, 0, SortOrder::Ascending);
        assert_eq!(spec.kind, SortKind::Text);
        // "" is missing and stays last. " " (0x20) sorts before "a".
        assert_eq!(apply(&doc, &rows, &spec), vec![1, 2, 0]);
        let desc = SortSpec {
            column: 0,
            order: SortOrder::Descending,
            kind: SortKind::Text,
        };
        assert_eq!(apply(&doc, &rows, &desc), vec![2, 1, 0]);
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
    fn zero_padded_ids_sort_as_distinct_text() {
        let doc = doc_from(b"id,pad\n10,a\n007,a\n7,a\n2,a\n");
        let rows = all_rows(&doc);
        let sorted = apply(&doc, &rows, &spec(&doc, 0, SortOrder::Ascending));
        // Not a numeric column: exact bytes are 007, 10, 2, 7.
        assert_eq!(sorted, vec![1, 0, 3, 2]);
    }

    #[test]
    fn equal_numbers_break_ties_on_exact_bytes() {
        let doc = doc_from(b"v,pad\n7,a\n\" 7\",a\n7,a\n");
        let rows = all_rows(&doc);
        let spec = SortSpec {
            column: 0,
            order: SortOrder::Ascending,
            kind: SortKind::Numeric,
        };
        // Both spellings are 7. The padded one sorts first by its bytes,
        // and the two exact "7" rows keep record order.
        assert_eq!(apply(&doc, &rows, &spec), vec![1, 0, 2]);
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

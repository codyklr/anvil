//! Query layer: search over a `ParsedDocument`.
//!
//! This module is GPUI-free and lives in `anvil_core` so it can be unit
//! tested without a window. The goal is a streaming, allocation-aware
//! search over the mmap:
//!
//! * We walk the record byte ranges in order, one record at a time.
//! * We do not materialise the whole document into owned strings.
//! * We use `memchr::memmem::Finder` for SIMD-accelerated byte scans.
//! * For case-insensitive matches we lowercase once on entry (the
//!   needle is short; per-record lowercasing is the cheap part because
//!   the record is short relative to the file).
//!
//! The result is a `Vec<u64>` of record indexes, ordered by record
//! position. Callers use this directly to highlight, jump to, or copy
//! matching rows; we deliberately do not return row copies because the
//! filtered / highlighted views all dereference into the original
//! `ParsedDocument`.

pub mod filter;
pub mod nl;
pub mod sort;
pub mod suggest;

use crate::parser::{Cells, ColumnIndex, ParsedDocument};

/// Per-call options for [`search`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchOptions {
    /// If `false` (the default), the needle matches case-insensitively
    /// (ASCII case folding). If `true`, matches are byte-exact.
    pub case_sensitive: bool,
    /// Restrict matches to a single column. `None` searches every
    /// column of every record (the common case).
    pub column: Option<usize>,
    /// Hard cap on matches. When `None`, search runs to completion.
    /// A cap is useful when the user types a single character and the
    /// candidate set explodes; the UI still has *some* progress to
    /// render rather than waiting for the whole file.
    pub max_matches: Option<u64>,
}

/// Search `doc` for `needle`, returning the matching record indexes in
/// record order. An empty needle matches nothing.
///
/// The algorithm:
/// 1. Build a lowercase copy of `needle` once when
///    `case_sensitive == false`. With case-sensitive search we use the
///    raw bytes directly.
/// 2. For each record in `[0, record_count)`, decode the bytes of each
///    cell (via `Cells`, which handles quoting / escapes). For every
///    cell that contains the needle (substring match), record the
///    `record_index`. Stop early when `max_matches` is reached.
/// 3. Return the collected indexes.
///
/// Complexity is `O(record_count × record_length)` in the worst case.
/// In practice most records are short, so the per-record cost is
/// effectively constant and the file-level scan is `O(file_length)`.
/// The byte scan is SIMD-accelerated by `memchr::memmem::Finder`.
pub fn search(doc: &ParsedDocument, needle: &str, options: &SearchOptions) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let record_count = doc.record_count();
    if record_count == 0 {
        return out;
    }
    let column_count = doc.column_count();

    // Build the SIMD finder once against the (possibly lowercased)
    // needle. The Finder borrows the buffer, so they live together.
    let lowercase_needle: Option<Vec<u8>> = if options.case_sensitive {
        None
    } else {
        Some(needle.bytes().map(ascii_lower).collect())
    };
    let needle_bytes: &[u8] = if options.case_sensitive {
        needle.as_bytes()
    } else {
        lowercase_needle.as_deref().unwrap_or(&[])
    };
    let finder = memchr::memmem::Finder::new(needle_bytes);
    let case_sensitive = options.case_sensitive;
    // Reused across every cell on the fold path, so a case-insensitive
    // search does not allocate per cell.
    let mut scratch: Vec<u8> = Vec::new();

    let cap = options.max_matches.unwrap_or(u64::MAX);

    for row in 0..record_count {
        if let Some(col) = options.column {
            // Column-scoped search: only inspect this column's cell.
            // Out-of-bounds columns silently produce no matches.
            if col >= column_count {
                continue;
            }
            let cell = match doc.cell(row, ColumnIndex(col)) {
                Some(c) => c,
                None => continue,
            };
            if cell_matches(cell.as_bytes(), &finder, case_sensitive, &mut scratch) {
                out.push(row);
                if out.len() as u64 >= cap {
                    return out;
                }
            }
        } else {
            // Whole-record search: walk every cell. The iterator stops
            // at the record's actual cell count, which is correct for
            // inconsistent-row files: short rows simply contribute
            // fewer cells, not errors.
            let Some(record_bytes) = doc.record_bytes(row) else {
                continue;
            };
            let iter = Cells::new(record_bytes, doc.meta.delimiter, doc.meta.quote);
            for cell in iter {
                if cell_matches(cell.as_ref(), &finder, case_sensitive, &mut scratch) {
                    out.push(row);
                    if out.len() as u64 >= cap {
                        return out;
                    }
                    break;
                }
            }
        }
    }

    out
}

/// Test one cell against a prebuilt `Finder`.
///
/// Case-sensitive cells are searched in place. For case-insensitive
/// search, an all-ASCII-lowercase cell (no uppercase bytes) is also
/// searched in place; otherwise the cell is folded into `scratch` once
/// and the SIMD finder runs over that. `scratch` is reused across cells,
/// so the fold path does not allocate per cell.
fn cell_matches(
    cell: &[u8],
    finder: &memchr::memmem::Finder<'_>,
    case_sensitive: bool,
    scratch: &mut Vec<u8>,
) -> bool {
    if cell.is_empty() {
        return false;
    }
    if case_sensitive || !cell.iter().any(u8::is_ascii_uppercase) {
        return finder.find(cell).is_some();
    }
    scratch.clear();
    scratch.extend(cell.iter().map(|b| ascii_lower(*b)));
    finder.find(scratch.as_slice()).is_some()
}

/// Returns the byte ranges of every occurrence of `needle` inside
/// `haystack`, suitable for substring highlighting. Empty when
/// `needle` does not occur or when either side is empty.
///
/// When `case_sensitive` is `true` the comparison is byte-exact and
/// the SIMD-accelerated `memchr::memmem::find_iter` is used. When
/// `false` ASCII case folding is used: we lowercase the entire
/// `haystack` into a single owned buffer and run `find_iter` against
/// that. The cost is one allocation per call; the caller (currently
/// only the grid renderer) drives this only for visible cells in
/// matched rows, so the cost stays bounded.
///
/// The ranges are in `&[u8]` byte coordinates; the caller is
/// responsible for translating them into whatever character /
/// unicode coordinates the renderer requires.
pub fn find_ranges(
    haystack: &[u8],
    needle: &[u8],
    case_sensitive: bool,
) -> Vec<std::ops::Range<usize>> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }
    if case_sensitive {
        memchr::memmem::find_iter(haystack, needle)
            .map(|start| start..start + needle.len())
            .collect()
    } else {
        let lowered: Vec<u8> = haystack.iter().map(|b| ascii_lower(*b)).collect();
        memchr::memmem::find_iter(&lowered, needle)
            .map(|start| start..start + needle.len())
            .collect()
    }
}

/// Does `haystack` contain `needle` as a substring, comparing
/// case-insensitively over ASCII bytes? Used by the filter's `contains`
/// operator (`query::filter::contains_ci`). For typical cells this is a
/// short scan; the search path uses a reusable scratch buffer instead.
pub(crate) fn ascii_contains_lower(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    let n = needle.len();
    let mut buf = [0u8; 256];
    if n > buf.len() {
        let lowered: Vec<u8> = haystack.iter().map(|b| ascii_lower(*b)).collect();
        return memchr::memmem::find(&lowered, needle).is_some();
    }
    for (i, b) in haystack.iter().take(n).enumerate() {
        buf[i] = ascii_lower(*b);
    }
    if buf[..n] == needle[..] {
        return true;
    }
    for &b in haystack.iter().skip(n) {
        for j in 0..(n - 1) {
            buf[j] = buf[j + 1];
        }
        buf[n - 1] = ascii_lower(b);
        if buf[..n] == needle[..] {
            return true;
        }
    }
    false
}

/// ASCII-only lowercasing. Bytes outside A–Z are returned unchanged.
#[inline]
fn ascii_lower(b: u8) -> u8 {
    if b.is_ascii_uppercase() {
        b + 32
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_opened;
    use crate::parser::ParsedDocument;
    use crate::source::local::open_local;

    fn parse_str(s: &str) -> ParsedDocument {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("data.csv");
        std::fs::write(&path, s.as_bytes()).expect("write");
        let opened = open_local(&path).expect("open");
        index_opened(opened)
    }

    #[test]
    fn empty_needle_returns_no_matches() {
        let doc = parse_str("a,b\n1,2\n3,4\n");
        let opts = SearchOptions::default();
        let r = search(&doc, "", &opts);
        assert!(r.is_empty());
    }

    #[test]
    fn single_match() {
        let doc = parse_str("name,city\nalice,paris\nbob,london\n");
        let opts = SearchOptions::default();
        let r = search(&doc, "alice", &opts);
        assert_eq!(r, vec![0]);
    }

    #[test]
    fn multiple_matches_in_record_order() {
        let doc = parse_str("a,b\nfoo,bar\nbaz,qux\nfoo,zip\n");
        let opts = SearchOptions::default();
        let r = search(&doc, "foo", &opts);
        assert_eq!(r, vec![0, 2]);
    }

    #[test]
    fn case_insensitive_default_matches_uppercase_data() {
        let doc = parse_str("a,b\nFOO,bar\nfoo,BAR\n");
        let opts = SearchOptions::default();
        let r = search(&doc, "foo", &opts);
        assert_eq!(r, vec![0, 1]);
    }

    #[test]
    fn case_sensitive_only_matches_exact_case() {
        let doc = parse_str("a,b\nFOO,bar\nfoo,BAR\n");
        let opts = SearchOptions {
            case_sensitive: true,
            ..Default::default()
        };
        let r = search(&doc, "foo", &opts);
        assert_eq!(r, vec![1]);
    }

    #[test]
    fn column_scoped_search() {
        let doc = parse_str("name,city\nalice,paris\nbob,paris\n");
        let opts = SearchOptions {
            column: Some(1),
            ..Default::default()
        };
        let r = search(&doc, "paris", &opts);
        assert_eq!(r, vec![0, 1]);
        let r0 = search(
            &doc,
            "alice",
            &SearchOptions {
                column: Some(0),
                ..Default::default()
            },
        );
        assert_eq!(r0, vec![0]);
    }

    #[test]
    fn column_scope_out_of_bounds_returns_empty() {
        let doc = parse_str("a,b\n1,2\n");
        let opts = SearchOptions {
            column: Some(99),
            ..Default::default()
        };
        let r = search(&doc, "1", &opts);
        assert!(r.is_empty());
    }

    #[test]
    fn quoted_cells_are_searched() {
        let doc = parse_str("a,b\n\"hello, world\",x\nfoo,\"hello, world\"\n");
        let opts = SearchOptions::default();
        let r = search(&doc, "hello, world", &opts);
        assert_eq!(r, vec![0, 1]);
    }

    #[test]
    fn max_matches_caps_results() {
        let doc = parse_str("a,b\nfoo,1\nfoo,2\nfoo,3\nfoo,4\nfoo,5\n");
        let opts = SearchOptions {
            max_matches: Some(3),
            ..Default::default()
        };
        let r = search(&doc, "foo", &opts);
        assert_eq!(r, vec![0, 1, 2]);
    }

    #[test]
    fn no_match_returns_empty() {
        let doc = parse_str("a,b\n1,2\n3,4\n");
        let opts = SearchOptions::default();
        let r = search(&doc, "zzz", &opts);
        assert!(r.is_empty());
    }

    #[test]
    fn empty_document_returns_empty() {
        // header-only, no records
        let doc = parse_str("a,b\n");
        let opts = SearchOptions::default();
        let r = search(&doc, "x", &opts);
        assert!(r.is_empty());
    }

    #[test]
    fn case_insensitive_matches_mixed_case_needle() {
        let doc = parse_str("a,b\nfoo,1\n");
        let opts = SearchOptions::default();
        assert_eq!(search(&doc, "FOO", &opts), vec![0]);
        assert_eq!(search(&doc, "Foo", &opts), vec![0]);
        assert_eq!(search(&doc, "fOo", &opts), vec![0]);
    }

    #[test]
    fn ascii_lower_helper_basic() {
        assert_eq!(ascii_lower(b'A'), b'a');
        assert_eq!(ascii_lower(b'Z'), b'z');
        assert_eq!(ascii_lower(b'a'), b'a');
        assert_eq!(ascii_lower(b'1'), b'1');
        assert_eq!(ascii_lower(b','), b',');
        assert_eq!(ascii_lower(0xC3), 0xC3);
    }

    // ---- find_ranges -----------------------------------------------------

    #[test]
    fn find_ranges_case_sensitive_single_match() {
        let r = find_ranges(b"hello world", b"world", true);
        assert_eq!(r, vec![6..11]);
    }

    #[test]
    fn find_ranges_case_sensitive_multiple_matches() {
        let r = find_ranges(b"foo bar foo baz foo", b"foo", true);
        assert_eq!(r, vec![0..3, 8..11, 16..19]);
    }

    #[test]
    fn find_ranges_case_sensitive_no_match() {
        assert!(find_ranges(b"hello", b"world", true).is_empty());
    }

    #[test]
    fn find_ranges_case_insensitive_matches_mixed_case() {
        let r = find_ranges(b"FOO foo FoO", b"foo", false);
        assert_eq!(r, vec![0..3, 4..7, 8..11]);
    }

    #[test]
    fn find_ranges_case_sensitive_respects_case() {
        let r = find_ranges(b"FOO foo", b"foo", true);
        assert_eq!(r, vec![4..7]);
    }

    #[test]
    fn find_ranges_empty_needle_returns_no_ranges() {
        let r = find_ranges(b"hello", b"", true);
        assert!(r.is_empty());
    }

    #[test]
    fn find_ranges_needle_longer_than_haystack_returns_no_ranges() {
        let r = find_ranges(b"hi", b"hello", true);
        assert!(r.is_empty());
    }

    #[test]
    fn find_ranges_empty_haystack_returns_no_ranges() {
        let r = find_ranges(b"", b"foo", true);
        assert!(r.is_empty());
    }

    #[test]
    fn find_ranges_overlapping_matches() {
        // "aaaa" has two occurrences of "aa": positions 0..2 and 2..4.
        // memchr's find_iter does not return overlapping matches, but
        // for typical search needles (longer than 1 byte) this case
        // is rare in real CSV data. We assert the documented
        // behaviour: non-overlapping.
        let r = find_ranges(b"aaaa", b"aa", true);
        assert_eq!(r, vec![0..2, 2..4]);
    }
}

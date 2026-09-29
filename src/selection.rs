//! Selection model for the grid.
//!
//! A `Selection` records the cursor cell and (optionally) an anchor cell
//! so that click-and-drag and shift-click can build rectangular ranges
//! from a starting point to the cursor. The model is independent of
//! GPUI: it owns no pixels and no elements, only `(record, column)` pairs.
//! This lets the same selection logic be exercised by unit tests without
//! a window.
//!
//! Invariants
//! ----------
//! * `cursor` and `anchor` are stored as `(record_index, column_index)`. A
//!   `None` selection means "nothing selected"; otherwise `cursor` is
//!   always present.
//! * `anchor` is `None` when there is no range; the moment the user
//!   extends the selection (shift-click, shift-arrow, click-and-drag),
//!   `anchor` becomes the cell at which the extension started.
//! * A `Selection::single(point)` is a degenerate range: cursor ==
//!   anchor.
//! * `normalize` returns the top-left and bottom-right corners of the
//!   selection rectangle in row-major order so the renderer can iterate
//!   without comparing coordinates by hand.
//!
//! These operations are deliberately `O(1)`. Cell-level iteration (which
//! is needed for copy / search-hit-testing / nav) is the renderer's
//! responsibility; this type only decides whether a given `(row, col)`
//! is inside the selection.

use crate::parser::ColumnIndex;

/// A logical grid coordinate. `(record_index, column_index)` with the
/// same conventions used everywhere else in the data side
/// (`record_index` is 0-based, `column_index` is 0-based, both
/// saturating at `u64::MAX` / `usize::MAX`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellPos {
    pub record_index: u64,
    pub column_index: usize,
}

impl CellPos {
    pub const fn new(record_index: u64, column_index: usize) -> Self {
        Self {
            record_index,
            column_index,
        }
    }
}

/// The selection state. `Option`-bearing so that "nothing selected" is
/// expressible without inventing a sentinel row/column.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    cursor: Option<CellPos>,
    anchor: Option<CellPos>,
}

impl Selection {
    pub fn empty() -> Self {
        Self::default()
    }

    /// A single-cell selection with no range yet.
    pub fn single(pos: CellPos) -> Self {
        Self {
            cursor: Some(pos),
            anchor: Some(pos),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cursor.is_none()
    }

    pub fn cursor(&self) -> Option<CellPos> {
        self.cursor
    }

    pub fn anchor(&self) -> Option<CellPos> {
        self.anchor
    }

    /// Replace the selection with a single cell. Discards any anchor.
    pub fn set(&mut self, pos: CellPos) {
        self.cursor = Some(pos);
        self.anchor = Some(pos);
    }

    /// Clear the selection entirely. After this call `is_empty()` is
    /// `true` and `cursor()` / `anchor()` are `None`.
    pub fn clear(&mut self) {
        self.cursor = None;
        self.anchor = None;
    }

    /// Move the cursor to `pos` while keeping the anchor fixed. This is
    /// what shift-arrow and shift-click do: the user extends an existing
    /// selection from a fixed starting cell to the new cursor.
    ///
    /// If the selection was empty, this is equivalent to `set(pos)`.
    pub fn extend_to(&mut self, pos: CellPos) {
        match (self.cursor, self.anchor) {
            (Some(cur), Some(anc)) => {
                if cur == anc {
                    // Was a single-cell selection; promote to a range.
                    self.cursor = Some(pos);
                    self.anchor = Some(anc);
                } else {
                    self.cursor = Some(pos);
                }
            }
            _ => self.set(pos),
        }
    }

    /// Move the cursor by `(row_delta, col_delta)` while preserving the
    /// anchor. Saturating arithmetic: negative deltas that would
    /// underflow clamp at zero, not at `u64::MAX`.
    ///
    /// The `record_count` parameter is the total number of data rows;
    /// the resulting row index is clamped to `[0, record_count - 1]`.
    /// The `column_count` parameter is the number of columns; the
    /// resulting column is clamped to `[0, column_count - 1]`. Empty
    /// documents (zero rows or columns) clamp to `(0, 0)` rather than
    /// panicking.
    pub fn move_cursor(
        &mut self,
        row_delta: i64,
        col_delta: i64,
        column_count: usize,
        record_count: u64,
        extend: bool,
    ) {
        let cur = self.cursor.unwrap_or(CellPos::new(0, 0));
        let row_max = record_count.saturating_sub(1) as i64;
        let row = clamp_signed(cur.record_index as i64 + row_delta, 0, row_max) as u64;
        let col_max = if column_count == 0 {
            0
        } else {
            (column_count - 1) as i64
        };
        let col = clamp_signed(cur.column_index as i64 + col_delta, 0, col_max) as usize;
        let next = CellPos::new(row, col);
        if extend {
            // Preserve the original anchor so the user can grow the
            // range. If there was no anchor yet, anchor at the current
            // cursor so the very first shift-arrow makes a real range.
            let new_anchor = self.anchor.unwrap_or(cur);
            self.cursor = Some(next);
            self.anchor = Some(new_anchor);
        } else {
            self.set(next);
        }
    }

    /// Return the normalized top-left and bottom-right corners of the
    /// selection rectangle. `None` when nothing is selected. The
    /// rectangle is inclusive on both ends.
    pub fn normalize(&self) -> Option<(CellPos, CellPos)> {
        let cur = self.cursor?;
        let anc = self.anchor.unwrap_or(cur);
        let top = cur.record_index.min(anc.record_index);
        let bot = cur.record_index.max(anc.record_index);
        let left = cur.column_index.min(anc.column_index);
        let right = cur.column_index.max(anc.column_index);
        Some((CellPos::new(top, left), CellPos::new(bot, right)))
    }

    /// Is `(record_index, column_index)` strictly inside the normalized
    /// rectangle?
    pub fn contains(&self, record_index: u64, column_index: usize) -> bool {
        let Some((tl, br)) = self.normalize() else {
            return false;
        };
        record_index >= tl.record_index
            && record_index <= br.record_index
            && column_index >= tl.column_index
            && column_index <= br.column_index
    }

    /// Convert a column index to a `ColumnIndex` for `ParsedDocument`.
    pub fn cursor_column(&self) -> Option<ColumnIndex> {
        self.cursor.map(|c| ColumnIndex(c.column_index))
    }

    /// Did the user actually pick a range (cursor != anchor)?
    pub fn is_range(&self) -> bool {
        match (self.cursor, self.anchor) {
            (Some(c), Some(a)) => c != a,
            _ => false,
        }
    }

    /// Number of data cells inside the selection rectangle. Zero when
    /// the selection is empty. Useful for a "copied N cells" status
    /// message and for tests.
    pub fn cell_count(&self) -> u64 {
        let Some((tl, br)) = self.normalize() else {
            return 0;
        };
        let rows = br.record_index - tl.record_index + 1;
        let cols = br.column_index as u64 - tl.column_index as u64 + 1;
        rows * cols
    }

    /// Render the selection as a TSV-formatted `String`, using
    /// `cell_at` to look up individual cell values. Each output row is
    /// cells joined by `\t`; rows are joined by the platform's
    /// standard line ending (LF on Unix, CRLF on Windows).
    ///
    /// Cells that contain a tab, newline, carriage return, or a
    /// double-quote are wrapped in double-quotes with embedded
    /// double-quotes doubled (RFC 4180-style). This preserves the
    /// content faithfully for paste-into-spreadsheet destinations
    /// (Excel, Numbers, Sheets) which all understand quoted TSV.
    ///
    /// Empty cells are emitted as the empty string — no quotes. A cell
    /// lookup that returns `None` (out-of-bounds row/column) is also
    /// emitted as the empty string.
    ///
    /// An empty selection produces an empty string.
    pub fn copy_text_with<F>(&self, cell_at: F) -> String
    where
        F: Fn(u64, usize) -> Option<String>,
    {
        let Some((tl, br)) = self.normalize() else {
            return String::new();
        };
        let line_ending = if cfg!(windows) { "\r\n" } else { "\n" };
        let mut out = String::new();
        for row in tl.record_index..=br.record_index {
            for col in tl.column_index..=br.column_index {
                if col > tl.column_index {
                    out.push('\t');
                }
                let raw = cell_at(row, col).unwrap_or_default();
                push_tsv_cell(&mut out, &raw);
            }
            out.push_str(line_ending);
        }
        out
    }
}

/// Push `cell` into `out` with TSV escaping applied. If the cell has
/// no special characters we emit it raw; otherwise we quote it and
/// double embedded quotes per RFC 4180.
fn push_tsv_cell(out: &mut String, cell: &str) {
    let needs_quote = cell.contains(['\t', '\n', '\r', '"']);
    if !needs_quote {
        out.push_str(cell);
        return;
    }
    out.push('"');
    for ch in cell.chars() {
        if ch == '"' {
            out.push('"');
            out.push('"');
        } else {
            out.push(ch);
        }
    }
    out.push('"');
}

fn clamp_signed(value: i64, lo: i64, hi: i64) -> i64 {
    if value < lo {
        lo
    } else if value > hi {
        hi
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_selection_has_no_cursor() {
        let s = Selection::empty();
        assert!(s.is_empty());
        assert_eq!(s.cursor(), None);
        assert_eq!(s.anchor(), None);
        assert!(s.normalize().is_none());
    }

    #[test]
    fn set_creates_single_cell_selection() {
        let mut s = Selection::empty();
        s.set(CellPos::new(3, 2));
        assert!(!s.is_empty());
        assert_eq!(s.cursor(), Some(CellPos::new(3, 2)));
        assert_eq!(s.anchor(), Some(CellPos::new(3, 2)));
        assert!(!s.is_range());
        assert_eq!(
            s.normalize(),
            Some((CellPos::new(3, 2), CellPos::new(3, 2)))
        );
    }

    #[test]
    fn extend_to_grows_range_from_anchor() {
        let mut s = Selection::empty();
        s.set(CellPos::new(2, 1));
        s.extend_to(CellPos::new(2, 3));
        assert_eq!(s.anchor(), Some(CellPos::new(2, 1)));
        assert_eq!(s.cursor(), Some(CellPos::new(2, 3)));
        assert!(s.is_range());
        assert_eq!(
            s.normalize(),
            Some((CellPos::new(2, 1), CellPos::new(2, 3)))
        );
    }

    #[test]
    fn extend_to_normalizes_when_anchor_is_after_cursor() {
        let mut s = Selection::empty();
        s.set(CellPos::new(5, 5));
        s.extend_to(CellPos::new(2, 1));
        assert_eq!(
            s.normalize(),
            Some((CellPos::new(2, 1), CellPos::new(5, 5)))
        );
    }

    #[test]
    fn move_cursor_clamps_row_and_column() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        // Move off the top-left corner.
        s.move_cursor(-10, -10, 4, 100, false);
        assert_eq!(s.cursor(), Some(CellPos::new(0, 0)));
        // Move past the bottom-right corner. The row index clamps to
        // record_count - 1, the column to column_count - 1.
        s.move_cursor(1000, 1000, 4, 100, false);
        assert_eq!(s.cursor(), Some(CellPos::new(99, 3)));
    }

    #[test]
    fn move_cursor_with_extension_keeps_anchor() {
        let mut s = Selection::empty();
        s.set(CellPos::new(2, 2));
        s.move_cursor(0, 1, 5, 10, true);
        assert_eq!(s.anchor(), Some(CellPos::new(2, 2)));
        assert_eq!(s.cursor(), Some(CellPos::new(2, 3)));
        assert!(s.is_range());
    }

    #[test]
    fn move_cursor_with_extension_from_empty_uses_start_as_anchor() {
        let mut s = Selection::empty();
        s.move_cursor(0, 1, 5, 10, true);
        assert_eq!(s.cursor(), Some(CellPos::new(0, 1)));
        // The anchor should be where the user "was" — the default origin
        // (0,0). After this first extend the range is (0,0)-(0,1).
        assert_eq!(s.anchor(), Some(CellPos::new(0, 0)));
        assert!(s.is_range());
    }

    #[test]
    fn contains_respects_normalized_rectangle() {
        let mut s = Selection::empty();
        s.set(CellPos::new(5, 5));
        s.extend_to(CellPos::new(2, 2));
        assert!(s.contains(2, 2));
        assert!(s.contains(5, 5));
        assert!(s.contains(3, 4));
        assert!(!s.contains(1, 1));
        assert!(!s.contains(6, 6));
        assert!(!s.contains(3, 6));
    }

    #[test]
    fn clear_resets_everything() {
        let mut s = Selection::empty();
        s.set(CellPos::new(2, 2));
        s.extend_to(CellPos::new(5, 5));
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.cursor(), None);
        assert_eq!(s.anchor(), None);
    }

    #[test]
    fn zero_column_count_does_not_panic() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        // column_count=0 should clamp to column 0, never panic.
        s.move_cursor(0, 1, 0, 10, false);
        assert_eq!(s.cursor(), Some(CellPos::new(0, 0)));
    }

    #[test]
    fn zero_records_clamps_row() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        s.move_cursor(1, 0, 5, 0, false);
        assert_eq!(s.cursor(), Some(CellPos::new(0, 0)));
    }

    // ---- copy_text / TSV formatter -------------------------------------

    #[test]
    fn cell_count_is_zero_for_empty_selection() {
        let s = Selection::empty();
        assert_eq!(s.cell_count(), 0);
    }

    #[test]
    fn cell_count_single_cell() {
        let mut s = Selection::empty();
        s.set(CellPos::new(3, 5));
        assert_eq!(s.cell_count(), 1);
    }

    #[test]
    fn cell_count_2x3_rectangle() {
        let mut s = Selection::empty();
        s.set(CellPos::new(2, 4));
        s.extend_to(CellPos::new(3, 6));
        assert_eq!(s.cell_count(), 2 * 3);
    }

    #[test]
    fn cell_count_normalizes_when_anchor_after_cursor() {
        let mut s = Selection::empty();
        s.set(CellPos::new(5, 5));
        s.extend_to(CellPos::new(1, 1));
        assert_eq!(s.cell_count(), 5 * 5);
    }

    #[test]
    fn copy_text_empty_selection_returns_empty_string() {
        let s = Selection::empty();
        let text = s.copy_text_with(|_, _| Some("x".into()));
        assert_eq!(text, "");
    }

    #[test]
    fn copy_text_single_cell_emits_one_line_with_trailing_line_ending() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        let text = s.copy_text_with(|_, _| Some("hello".into()));
        let expected_ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("hello{expected_ending}"));
    }

    #[test]
    fn copy_text_row_range_emits_rows_separated_by_line_ending() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        s.extend_to(CellPos::new(1, 2));
        let text = s.copy_text_with(|r, c| Some(format!("r{r}c{c}")));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(
            text,
            format!("r0c0\tr0c1\tr0c2{ending}r1c0\tr1c1\tr1c2{ending}")
        );
    }

    #[test]
    fn copy_text_out_of_bounds_cell_is_empty() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        s.extend_to(CellPos::new(1, 1));
        let text = s.copy_text_with(|r, c| {
            // Only (0, 0) exists; everything else is "out of bounds".
            if r == 0 && c == 0 {
                Some("real".into())
            } else {
                None
            }
        });
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("real\t{ending}\t{ending}"));
    }

    #[test]
    fn copy_text_quotes_cells_with_tabs() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        let text = s.copy_text_with(|_, _| Some("col1\tcol2".into()));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("\"col1\tcol2\"{ending}"));
    }

    #[test]
    fn copy_text_quotes_cells_with_newlines() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        let text = s.copy_text_with(|_, _| Some("line1\nline2".into()));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("\"line1\nline2\"{ending}"));
    }

    #[test]
    fn copy_text_doubles_embedded_double_quotes() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        let text = s.copy_text_with(|_, _| Some(r#"say "hi""#.into()));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("\"say \"\"hi\"\"\"{ending}"));
    }

    #[test]
    fn copy_text_does_not_quote_plain_cells() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        s.extend_to(CellPos::new(0, 1));
        let text = s.copy_text_with(|_, c| Some(if c == 0 { "a".into() } else { "b".into() }));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("a\tb{ending}"));
    }

    #[test]
    fn copy_text_quotes_carriage_return() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        let text = s.copy_text_with(|_, _| Some("with\rcr".into()));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("\"with\rcr\"{ending}"));
    }

    #[test]
    fn copy_text_column_range_emits_columns_with_tabs() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 1));
        s.extend_to(CellPos::new(2, 3));
        let text = s.copy_text_with(|r, c| Some(format!("r{r}c{c}")));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(
            text,
            format!("r0c1\tr0c2\tr0c3{ending}r1c1\tr1c2\tr1c3{ending}r2c1\tr2c2\tr2c3{ending}")
        );
    }
}

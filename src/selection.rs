//! Selection model for the grid.
//!
//! A `Selection` records the cursor cell and (optionally) an anchor cell
//! so that click-and-drag and shift-click can build rectangular ranges
//! from a starting point to the cursor. The model is independent of
//! GPUI: it owns no pixels and no elements, only `(row, column)` pairs.
//! This lets the same selection logic be exercised by unit tests without
//! a window.
//!
//! Rows are **display positions**, not record indexes: row `i` is the
//! `i`-th row the user can see, after any active filter or sort. Without
//! a filter or sort the two coincide. Keeping the selection in display
//! space means a drag always highlights and copies the visible block,
//! and the caller resolves positions to record indexes only when it
//! reads or copies a cell.
//!
//! Invariants
//! ----------
//! * `cursor` and `anchor` are stored as `(display_row, column_index)`. A
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

/// A logical grid coordinate. `(display_row, column_index)` with
/// `display_row` the 0-based position in the rendered view (so it walks
/// the filtered/sorted order) and `column_index` 0-based. Both saturate
/// at `u64::MAX` / `usize::MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellPos {
    pub row: u64,
    pub column_index: usize,
}

impl CellPos {
    pub const fn new(row: u64, column_index: usize) -> Self {
        Self { row, column_index }
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

    /// Select the entire view: anchor at `(0, 0)`, cursor at
    /// `(row_count - 1, column_count - 1)`. An empty view clears the
    /// selection instead of leaving a dangling cursor.
    ///
    /// `row_count` is the number of rows the user can currently see:
    /// the filtered/sorted view when one is active, otherwise every
    /// record. Positions are display positions, so the rectangle covers
    /// exactly the visible rows.
    pub fn select_all(&mut self, row_count: u64, column_count: usize) {
        if row_count == 0 || column_count == 0 {
            self.clear();
            return;
        }
        self.anchor = Some(CellPos::new(0, 0));
        self.cursor = Some(CellPos::new(row_count - 1, column_count - 1));
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
    /// The `row_count` parameter is the number of display rows in the
    /// current view; the resulting row position is clamped to
    /// `[0, row_count - 1]`. The `column_count` parameter is the number
    /// of columns; the resulting column is clamped to
    /// `[0, column_count - 1]`. Empty views (zero rows or columns) clamp
    /// to `(0, 0)` rather than panicking.
    pub fn move_cursor(
        &mut self,
        row_delta: i64,
        col_delta: i64,
        column_count: usize,
        row_count: u64,
        extend: bool,
    ) {
        let cur = self.cursor.unwrap_or(CellPos::new(0, 0));
        let row_max = row_count.saturating_sub(1) as i64;
        // `row_delta` / `col_delta` may be `i64::MIN` / `i64::MAX`
        // (Home / End, Cmd/Ctrl+Home / End). Saturating arithmetic
        // keeps those extremes well-defined instead of overflowing.
        let row = clamp_signed((cur.row as i64).saturating_add(row_delta), 0, row_max) as u64;
        let col_max = if column_count == 0 {
            0
        } else {
            (column_count - 1) as i64
        };
        let col = clamp_signed(
            (cur.column_index as i64).saturating_add(col_delta),
            0,
            col_max,
        ) as usize;
        self.apply_destination(CellPos::new(row, col), extend);
    }

    /// Move the cursor to the adjacent cell in row-major order,
    /// wrapping across row boundaries and around the document ends.
    /// `forward == true` is Tab; `false` is Shift-Tab.
    ///
    /// Does nothing when the view has zero rows or columns. The wrap
    /// stays inside the view because the total cell count is
    /// `row_count * column_count`.
    pub fn move_cell(&mut self, forward: bool, column_count: usize, row_count: u64, extend: bool) {
        if column_count == 0 || row_count == 0 {
            return;
        }
        let cols = column_count as u64;
        let total = row_count.saturating_mul(cols);
        if total == 0 {
            return;
        }
        let cur = self.cursor.unwrap_or(CellPos::new(0, 0));
        let clamped_row = cur.row.min(row_count - 1);
        let clamped_col = (cur.column_index as u64).min(cols - 1);
        let linear = clamped_row * cols + clamped_col;
        let next_linear = if forward {
            (linear + 1) % total
        } else {
            (linear + total - 1) % total
        };
        let next = CellPos::new(next_linear / cols, (next_linear % cols) as usize);
        self.apply_destination(next, extend);
    }

    /// Commit a computed destination cell. When `extend` is set the
    /// original anchor is preserved (anchoring at the current cursor
    /// when there was no anchor yet, so the first shift-arrow makes a
    /// real range); otherwise the selection becomes a single cell.
    fn apply_destination(&mut self, next: CellPos, extend: bool) {
        if extend {
            let cur = self.cursor.unwrap_or(CellPos::new(0, 0));
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
        let top = cur.row.min(anc.row);
        let bot = cur.row.max(anc.row);
        let left = cur.column_index.min(anc.column_index);
        let right = cur.column_index.max(anc.column_index);
        Some((CellPos::new(top, left), CellPos::new(bot, right)))
    }

    /// Is `(row, column)` strictly inside the normalized rectangle?
    pub fn contains(&self, row: u64, column_index: usize) -> bool {
        let Some((tl, br)) = self.normalize() else {
            return false;
        };
        row >= tl.row
            && row <= br.row
            && column_index >= tl.column_index
            && column_index <= br.column_index
    }

    /// Clamp the cursor and anchor into `[0, row_count - 1]`. A zero-row
    /// view clears the selection. Called when a filter or sort changes
    /// the visible row set so a stale position never exceeds the view.
    pub fn clamp_to_rows(&mut self, row_count: u64) {
        if row_count == 0 {
            self.clear();
            return;
        }
        let max = row_count - 1;
        if let Some(c) = self.cursor {
            self.cursor = Some(CellPos::new(c.row.min(max), c.column_index));
        }
        if let Some(a) = self.anchor {
            self.anchor = Some(CellPos::new(a.row.min(max), a.column_index));
        }
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
        let rows = br.row - tl.row + 1;
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
    /// Empty cells are emitted as the empty string - no quotes. A cell
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
        for row in tl.row..=br.row {
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

    /// Render the selection as TSV for the current view. The selection's
    /// rows are *display positions*; `view_rows` is the mapping from a
    /// display position to a record index. Rows are emitted in view order
    /// and the column range is the selection's normalized column range.
    /// A position past the end of `view_rows` (a stale selection after
    /// the view shrank) is skipped.
    pub fn copy_view_text_with<F>(&self, view_rows: &[u64], cell_at: F) -> String
    where
        F: Fn(u64, usize) -> Option<String>,
    {
        let Some((tl, br)) = self.normalize() else {
            return String::new();
        };
        let line_ending = if cfg!(windows) { "\r\n" } else { "\n" };
        if view_rows.is_empty() {
            return String::new();
        }
        let last = (br.row as usize).min(view_rows.len() - 1);
        if (tl.row as usize) > last {
            return String::new();
        }
        let mut out = String::new();
        for position in (tl.row as usize)..=last {
            let Some(&row) = view_rows.get(position) else {
                break;
            };
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

    /// Number of cells the selection covers in the current view: the
    /// normalized row range intersected with `view_rows`, times the
    /// column range. Used for the "copied N cells" status message so it
    /// matches what [`copy_view_text_with`](Self::copy_view_text_with)
    /// writes.
    pub fn view_cell_count(&self, view_rows: &[u64]) -> u64 {
        let Some((tl, br)) = self.normalize() else {
            return 0;
        };
        let cols = br.column_index as u64 - tl.column_index as u64 + 1;
        if view_rows.is_empty() {
            return 0;
        }
        let last = (br.row as usize).min(view_rows.len() - 1);
        if (tl.row as usize) > last {
            return 0;
        }
        let rows = (last - tl.row as usize + 1) as u64;
        rows * cols
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
        // Move past the bottom-right corner. The row position clamps
        // to row_count - 1, the column to column_count - 1.
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
        // The anchor should be where the user "was" - the default origin
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

    #[test]
    fn move_cursor_with_extreme_delta_saturates() {
        // Cmd/Ctrl+Home/End pass `i64::MIN` / `i64::MAX` as the row
        // delta. This must not overflow; it must land on the first /
        // last row respectively.
        let mut s = Selection::empty();
        s.set(CellPos::new(5, 2));
        s.move_cursor(i64::MIN, 0, 4, 10, false);
        assert_eq!(s.cursor(), Some(CellPos::new(0, 2)));
        s.move_cursor(i64::MAX, 0, 4, 10, false);
        assert_eq!(s.cursor(), Some(CellPos::new(9, 2)));
    }

    #[test]
    fn move_cursor_with_extreme_column_delta_saturates() {
        // Home / End pass `i64::MIN` / `i64::MAX` as the column delta.
        let mut s = Selection::empty();
        s.set(CellPos::new(5, 2));
        s.move_cursor(0, i64::MIN, 4, 10, false);
        assert_eq!(s.cursor(), Some(CellPos::new(5, 0)));
        s.move_cursor(0, i64::MAX, 4, 10, false);
        assert_eq!(s.cursor(), Some(CellPos::new(5, 3)));
    }

    #[test]
    fn move_cell_advances_and_wraps_rows() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 2));
        // Last column of row 0 → first column of row 1.
        s.move_cell(true, 3, 3, false);
        assert_eq!(s.cursor(), Some(CellPos::new(1, 0)));
    }

    #[test]
    fn move_cell_backward_wraps_to_previous_row_end() {
        let mut s = Selection::empty();
        s.set(CellPos::new(1, 0));
        s.move_cell(false, 3, 3, false);
        assert_eq!(s.cursor(), Some(CellPos::new(0, 2)));
    }

    #[test]
    fn move_cell_wraps_around_document_ends() {
        let mut s = Selection::empty();
        s.set(CellPos::new(2, 2));
        s.move_cell(true, 3, 3, false);
        assert_eq!(s.cursor(), Some(CellPos::new(0, 0)));
        s.move_cell(false, 3, 3, false);
        assert_eq!(s.cursor(), Some(CellPos::new(2, 2)));
    }

    #[test]
    fn move_cell_from_empty_anchors_at_origin() {
        let mut s = Selection::empty();
        s.move_cell(true, 3, 3, true);
        assert_eq!(s.cursor(), Some(CellPos::new(0, 1)));
        assert_eq!(s.anchor(), Some(CellPos::new(0, 0)));
    }

    #[test]
    fn move_cell_no_op_on_empty_document() {
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        let mut zero_cols = s.clone();
        zero_cols.move_cell(true, 0, 5, false);
        assert_eq!(zero_cols, s);
        let mut zero_rows = s.clone();
        zero_rows.move_cell(true, 5, 0, false);
        assert_eq!(zero_rows, s);
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

    #[test]
    fn select_all_covers_the_whole_document() {
        let mut s = Selection::empty();
        s.select_all(3, 4);
        assert!(s.is_range());
        assert_eq!(s.anchor(), Some(CellPos::new(0, 0)));
        assert_eq!(s.cursor(), Some(CellPos::new(2, 3)));
        assert_eq!(s.cell_count(), 12);
    }

    #[test]
    fn select_all_on_empty_document_clears() {
        let mut s = Selection::empty();
        s.set(CellPos::new(1, 1));
        s.select_all(0, 4);
        assert!(s.is_empty());
        s.set(CellPos::new(1, 1));
        s.select_all(3, 0);
        assert!(s.is_empty());
    }

    #[test]
    fn copy_view_text_skips_positions_past_a_shorter_view() {
        let mut s = Selection::empty();
        s.select_all(3, 2);
        // View order: records 0, 2 and 4 are the only visible ones.
        let view = [0u64, 2, 4];
        let text = s.copy_view_text_with(&view, |r, c| Some(format!("r{r}c{c}")));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(
            text,
            format!("r0c0\tr0c1{ending}r2c0\tr2c1{ending}r4c0\tr4c1{ending}")
        );
        assert_eq!(s.view_cell_count(&view), 6);
    }

    #[test]
    fn copy_view_uses_positions_not_record_indexes() {
        // A sort permutation where display rows 0 and 1 are records 2
        // and 0. The selection covers the two visible rows, so copy must
        // emit exactly those two records, not the record rectangle
        // 0..=2.
        let mut s = Selection::empty();
        s.set(CellPos::new(0, 0));
        s.extend_to(CellPos::new(1, 1));
        let view = [2u64, 0, 1];
        let text = s.copy_view_text_with(&view, |r, c| Some(format!("r{r}c{c}")));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("r2c0\tr2c1{ending}r0c0\tr0c1{ending}"));
        assert_eq!(s.view_cell_count(&view), 4);
    }

    #[test]
    fn copy_view_clamps_a_stale_selection_to_the_view() {
        let mut s = Selection::empty();
        s.select_all(5, 1);
        let view = [0u64, 2, 4];
        let text = s.copy_view_text_with(&view, |r, _| Some(format!("r{r}")));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("r0{ending}r2{ending}r4{ending}"));
        assert_eq!(s.view_cell_count(&view), 3);
    }

    #[test]
    fn clamp_to_rows_clamps_cursor_and_anchor() {
        let mut s = Selection::empty();
        s.set(CellPos::new(8, 1));
        s.extend_to(CellPos::new(3, 2));
        s.clamp_to_rows(4);
        assert_eq!(s.cursor(), Some(CellPos::new(3, 2)));
        assert_eq!(s.anchor(), Some(CellPos::new(3, 1)));
        s.clamp_to_rows(0);
        assert!(s.is_empty());
    }

    #[test]
    fn copy_view_text_preserves_display_order() {
        let mut s = Selection::empty();
        s.select_all(3, 1);
        // A descending sort permutation of records 0, 1, 2.
        let view = [2u64, 1, 0];
        let text = s.copy_view_text_with(&view, |r, _| Some(format!("r{r}")));
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(text, format!("r2{ending}r1{ending}r0{ending}"));
    }
}

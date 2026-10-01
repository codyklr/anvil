//! Virtualized grid layout and row rendering.
//!
//! Vertical virtualization is provided by GPUI's `uniform_list`. Horizontal
//! virtualization is done here: `ColumnLayout` knows each column's pixel
//! width and offset, so a row only builds elements for the columns that are
//! actually visible. That keeps a 2,000-column file cheap to display.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use gpui::{
    div, prelude::*, px, rgb, HighlightStyle, Hsla, IntoElement, MouseDownEvent, MouseMoveEvent,
    Pixels, SharedString, StyledText,
};

use anvil_core::{
    exact_string, find_ranges, CellPos, Cells, ParsedDocument, SearchOptions, Selection,
    WarningKind,
};

use super::theme;

/// Precomputed horizontal geometry for a document.
pub struct ColumnLayout {
    pub widths: Vec<f32>,
    pub offsets: Vec<f32>,
    pub total: f32,
}

impl ColumnLayout {
    /// Build a layout by sampling up to `SAMPLE_ROWS` records per column.
    pub fn compute(doc: &ParsedDocument) -> Self {
        let n = doc.column_count();
        let mut widths = vec![f32::from(theme::DEFAULT_COLUMN_WIDTH); n];
        let sampled_rows = doc.record_count().min(SAMPLE_ROWS);
        let measured = n.min(theme::MAX_MEASURED_COLUMNS);
        let min_w = f32::from(theme::MIN_COLUMN_WIDTH);
        let pad = f32::from(theme::CELL_PAD_X) * 2.0;

        // Seed the measured columns with their header width.
        for (col, width) in widths.iter_mut().enumerate().take(measured) {
            *width = text_width(doc.meta.headers.get(col).map(String::as_str).unwrap_or(""));
        }
        // Update every column from a single forward pass per sampled row.
        // Calling `doc.cell` once per column re-walked the record from its
        // start for each column (O(columns^2) per row); one `Cells` pass is
        // O(record length). `text_width` itself is O(1) (it caps at 64
        // characters), so the decode dominates.
        for row in 0..sampled_rows {
            let Some(record) = doc.record_bytes(row) else {
                continue;
            };
            let iter = Cells::new(record, doc.meta.delimiter, doc.meta.quote);
            for (col, cell) in iter.enumerate().take(measured) {
                let w = text_width(std::str::from_utf8(cell.as_ref()).unwrap_or(""));
                if w > widths[col] {
                    widths[col] = w;
                }
            }
        }
        for width in widths.iter_mut().take(measured) {
            *width = (*width + pad).clamp(min_w, MAX_AUTO_WIDTH);
        }

        let mut offsets = Vec::with_capacity(n + 1);
        let mut acc = 0.0f32;
        for &w in &widths {
            offsets.push(acc);
            acc += w;
        }
        offsets.push(acc);

        Self {
            widths,
            offsets,
            total: acc,
        }
    }

    /// Rebuild a layout from an explicit width vector, recomputing the
    /// offsets and total. Used to restore per-tab widths after a tab
    /// switch and to apply a manual resize.
    pub fn from_widths(widths: Vec<f32>) -> Self {
        let mut offsets = Vec::with_capacity(widths.len() + 1);
        let mut acc = 0.0f32;
        for &w in &widths {
            offsets.push(acc);
            acc += w;
        }
        offsets.push(acc);
        Self {
            widths,
            offsets,
            total: acc,
        }
    }

    /// A copy with `column` set to `width`, clamped to
    /// `[MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH]`. Offsets and total are
    /// recomputed, so the header, body cells and horizontal scrollbar
    /// stay aligned.
    pub fn resized(&self, column: usize, width: f32) -> Self {
        let mut widths = self.widths.clone();
        if let Some(slot) = widths.get_mut(column) {
            *slot = width.clamp(
                f32::from(theme::MIN_COLUMN_WIDTH),
                f32::from(theme::MAX_COLUMN_WIDTH),
            );
        }
        Self::from_widths(widths)
    }

    /// Columns intersecting `[scroll_x, scroll_x + viewport_w)`.
    pub fn visible(&self, scroll_x: f32, viewport_w: f32) -> Range<usize> {
        let n = self.widths.len();
        if n == 0 || viewport_w <= 0.0 {
            return 0..0;
        }
        let left = scroll_x.max(0.0);
        let right = left + viewport_w;
        let start = match self
            .offsets
            .binary_search_by(|o| o.partial_cmp(&left).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) => i.min(n.saturating_sub(1)),
            Err(i) => i.saturating_sub(1),
        };
        let mut end = start;
        while end < n && self.offsets[end] < right {
            end += 1;
        }
        start..end
    }

    pub fn width_of(&self, col: usize) -> Pixels {
        px(self.widths.get(col).copied().unwrap_or(0.0))
    }

    pub fn offset_of(&self, col: usize) -> Pixels {
        px(self.offsets.get(col).copied().unwrap_or(0.0))
    }
}

/// Conservative estimate of a string's rendered width at our font size.
/// We deliberately avoid asking the text system here because this runs for
/// every sampled cell; a small over-estimate is fine.
fn text_width(s: &str) -> f32 {
    let mut units = 0.0f32;
    for c in s.chars().take(64) {
        units += if (c as u32) > 0x2E80 { 1.7 } else { 1.0 };
    }
    units * AVG_CHAR_WIDTH
}

const AVG_CHAR_WIDTH: f32 = 7.0;
const MAX_AUTO_WIDTH: f32 = 360.0;
const SAMPLE_ROWS: u64 = 200;

/// Upper bound on the visible-range cell-text cache. A tall viewport is
/// ~40 rows; 4096 entries covers ~100 columns of that. When a very wide
/// file pushes past the cap the cache is cleared rather than evicted:
/// scrolling then re-populates it, and the cap keeps memory bounded.
const CELL_TEXT_CACHE_CAP: usize = 4096;

/// Is `(record, column)` inside the inclusive `(top-left, bottom-right)`
/// rectangle? `None` is never inside.
fn rect_contains(rect: Option<(CellPos, CellPos)>, record: u64, column: usize) -> bool {
    match rect {
        Some((tl, br)) => {
            record >= tl.record_index
                && record <= br.record_index
                && column >= tl.column_index
                && column <= br.column_index
        }
        None => false,
    }
}

/// The immutable text a grid cell displays, as a `SharedString`.
///
/// `exact_string` borrows from the mmap for valid UTF-8 and allocates an
/// escaped `String` only when the bytes are not valid UTF-8.
/// `SharedString::new(text.as_ref())` then allocates a single `Arc<str>`
/// and copies the bytes once. Building it from `text.to_string()` (the
/// previous form) allocated the intermediate `String` *and* the
/// `Arc<str>`, copying every visible cell twice per frame. The visible
/// bytes are identical either way, including the escaped form used for
/// invalid UTF-8.
fn cell_shared_string(bytes: &[u8]) -> SharedString {
    let text = exact_string(bytes);
    SharedString::new(text.as_ref())
}

/// A read-only view over one document that the grid renders.
#[derive(Clone)]
pub struct GridModel {
    pub doc: Arc<ParsedDocument>,
    pub layout: Arc<ColumnLayout>,
    /// Record -> representative parse-warning kind, sorted by record and
    /// deduplicated to the most informative kind per row. Computed once
    /// at open because `ParseMeta.warnings` never changes for a
    /// document; the grid binary-searches it per visible row.
    pub warnings: Arc<[(u64, WarningKind)]>,
    /// Bounded cache of the immutable text for `(record, column)` cells
    /// shown in the visible range. `uniform_list` re-invokes the row
    /// renderer for every visible row each frame, and a scroll step only
    /// changes one or two of them, so the cache removes the per-cell
    /// `Arc<str>` allocation on the ~95% of cells that repeat. The text
    /// depends only on the document, so the cache is invalidated simply
    /// by building a new `GridModel` (which open / reindex already do).
    text_cache: Arc<Mutex<HashMap<(u64, usize), SharedString>>>,
}

impl GridModel {
    pub fn new(doc: Arc<ParsedDocument>) -> Self {
        let layout = ColumnLayout::compute(&doc);
        let warnings = Arc::from(cache_warning_rows(&doc));
        Self {
            doc,
            layout: Arc::new(layout),
            warnings,
            text_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn row_count(&self) -> u64 {
        self.doc.record_count()
    }

    /// The `SharedString` for one visible cell, consulting the bounded
    /// cache first. `bytes` must be the cell's bytes for `(record,
    /// column)`; the cache assumes they do not change for a document.
    fn cell_text(&self, record: u64, column: usize, bytes: &[u8]) -> SharedString {
        let mut cache = self
            .text_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = cache.get(&(record, column)) {
            return existing.clone();
        }
        let text = cell_shared_string(bytes);
        if cache.len() >= CELL_TEXT_CACHE_CAP {
            cache.clear();
        }
        cache.insert((record, column), text.clone());
        text
    }
}

/// Rank a warning kind by how much it explains about a row, so a row
/// that trips several checks shows its root cause (an unterminated
/// quote also makes a record very long, but the quote is the real
/// story). Header warnings have no `record_index` and are excluded from
/// the row index; the diagnostics panel still lists them.
fn warning_severity(kind: WarningKind) -> u8 {
    match kind {
        WarningKind::UnterminatedQuote => 3,
        WarningKind::VeryLongRecord => 2,
        WarningKind::InconsistentColumnCount => 1,
        WarningKind::EmptyHeader | WarningKind::DuplicateHeader => 0,
    }
}

/// Build the sorted, per-record warning index cached on `GridModel`.
fn cache_warning_rows(doc: &ParsedDocument) -> Vec<(u64, WarningKind)> {
    let mut rows: Vec<(u64, WarningKind)> = doc
        .meta
        .warnings
        .iter()
        .filter_map(|w| w.record_index.map(|r| (r, w.kind)))
        .collect();
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| warning_severity(b.1).cmp(&warning_severity(a.1)))
    });
    rows.dedup_by_key(|(record, _)| *record);
    rows
}

/// The representative warning kind for `record`, or `None`.
/// `rows` must be sorted by record (the `GridModel` cache is).
fn warning_kind_for(rows: &[(u64, WarningKind)], record: u64) -> Option<WarningKind> {
    rows.binary_search_by(|(r, _)| r.cmp(&record))
        .ok()
        .map(|i| rows[i].1)
}

/// Glyph shown in the row-number gutter for a warning row.
fn warning_glyph(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::UnterminatedQuote => "✖",
        _ => "⚠",
    }
}

/// Colour of the row-number gutter for a warning row. Quoting problems
/// are errors; the rest are warnings.
fn warning_color(kind: WarningKind) -> gpui::Rgba {
    match kind {
        WarningKind::UnterminatedQuote => theme::ERROR,
        _ => theme::WARN,
    }
}

/// The sticky header row. Renders only the visible column headers.
///
/// `scoped_column` is the column currently selected as the search
/// scope (set by Alt/Option-clicking a header); it is painted with a
/// persistent marker so the scope is visible even when the search bar
/// is closed. `sort` is `(column, descending)` for the active sort and
/// adds a direction arrow to that header. `on_header_click` receives
/// the clicked column and the mouse-down event so the view can tell a
/// plain click (sort) from a modifier click (search scope). It uses
/// the same `entity.update` closure pattern as the per-cell handlers.
pub fn render_header<F, G>(
    model: &GridModel,
    scroll_x: f32,
    viewport_w: f32,
    scoped_column: Option<usize>,
    sort: Option<(usize, bool)>,
    on_header_click: F,
    on_resize_start: G,
) -> impl IntoElement
where
    F: Fn(usize, &MouseDownEvent, &mut gpui::App) + Clone + 'static,
    G: Fn(usize, &MouseDownEvent, &mut gpui::App) + Clone + 'static,
{
    let vis = model.layout.visible(scroll_x, viewport_w);
    let pad = model.layout.offset_of(vis.start);
    let mut cells = Vec::with_capacity(vis.len());
    for col in vis {
        let mut label = model.doc.meta.column_label(col);
        if let Some((sort_col, descending)) = sort {
            if sort_col == col {
                label.push(' ');
                label.push(if descending { '▼' } else { '▲' });
            }
        }
        let click = on_header_click.clone();
        let mut cell = div()
            .relative()
            .flex_shrink_0()
            .w(model.layout.width_of(col))
            .px(theme::CELL_PAD_X)
            .truncate()
            .cursor_pointer()
            .debug_selector(move || format!("header-cell-{col}"))
            .text_color(theme::TEXT_STRONG)
            .hover(|s| s.bg(theme::BG_ROW_HOVER));
        if scoped_column == Some(col) {
            // Persistent marker for the search column scope.
            cell = cell.bg(theme::BG_MATCH_ACTIVE);
        }
        cell = cell.on_mouse_down(gpui::MouseButton::Left, move |ev, _window, app_cx| {
            click(col, ev, app_cx);
        });
        // A thin drag handle on the column's right edge. It claims the
        // press so the header click (sort / scope) does not also fire.
        let resize = on_resize_start.clone();
        cell = cell.child(
            div()
                .absolute()
                .right(px(0.))
                .top(px(0.))
                .h_full()
                .w(px(5.))
                .cursor_ew_resize()
                .hover(|s| s.bg(theme::ACCENT))
                .debug_selector(move || format!("header-resize-{col}"))
                .on_mouse_down(gpui::MouseButton::Left, move |ev, _window, app_cx| {
                    app_cx.stop_propagation();
                    resize(col, ev, app_cx);
                }),
        );
        cells.push(cell.child(label));
    }

    div()
        .flex()
        .flex_row()
        .items_center()
        .h(theme::HEADER_HEIGHT)
        .w_full()
        .bg(theme::BG_HEADER)
        .border_b_1()
        .border_color(theme::BORDER_STRONG)
        .text_xs()
        .child(
            // Row-number gutter's header.
            div()
                .flex_shrink_0()
                .w(theme::ROW_NUMBER_WIDTH)
                .px(theme::CELL_PAD_X)
                .text_color(theme::TEXT_DIM)
                .child("#"),
        )
        .child(
            div().flex_1().overflow_hidden().child(
                div()
                    .flex()
                    .flex_row()
                    .ml(px(-scroll_x))
                    .child(div().flex_shrink_0().w(pad))
                    .children(cells),
            ),
        )
}

/// Per-frame inputs shared by every row in a single render pass. Bundled
/// so `render_row` does not grow an unbounded argument list as more
/// per-frame state is added.
pub struct RowContext<'a> {
    pub selection: &'a Selection,
    pub scroll_x: f32,
    pub viewport_w: f32,
    /// Rectangle of the most recent copy, painted `theme::BG_COPIED`.
    pub copy_flash: Option<(CellPos, CellPos)>,
    /// Sorted list of record indexes that match the current search
    /// query. Empty when no search is active. Looked up by binary
    /// search for every visible row, so it stays cheap on 1M-row
    /// documents (~30 visible rows × ~7 probes = 200 binary searches
    /// per redraw).
    pub search_matches: &'a [u64],
    /// Index into `search_matches` of the active row, or `-1` when
    /// there is no active match (e.g. an empty result set). The active
    /// row is painted with `BG_MATCH_ACTIVE`; all other matches are
    /// `BG_MATCH`.
    pub search_active: i64,
    /// The needle the user typed. Empty when there is no search.
    /// Used to highlight the matched substring within each cell of a
    /// match row; rendering skips the substring scan when the row is
    /// not a match, so the cost is bounded to ~30 visible rows × ~6
    /// columns on the 1M-row fixture.
    pub search_query: &'a str,
    /// Search options that affect highlighting: `case_sensitive`
    /// selects the comparison, and `column` restricts highlighting to
    /// the scoped column. The data-side `find_ranges` still only
    /// decides byte ranges; the column gate lives in `should_highlight`
    /// so a match row does not light up columns outside the scope.
    pub search_options: SearchOptions,
    /// Record indexes that carry a parse warning, paired with a
    /// representative kind and sorted by record. Cached on `GridModel`;
    /// the row renderer binary-searches it to flag the gutter.
    pub warning_rows: &'a [(u64, WarningKind)],
}

/// Should matched text be highlighted in `column` under `options`?
/// `None` scope highlights every column; a set scope highlights only
/// the matching column (and the row tint still marks the whole row).
fn should_highlight(options: &SearchOptions, column: usize) -> bool {
    match options.column {
        None => true,
        Some(scoped) => scoped == column,
    }
}

/// Renders a single data row. `index` is the 0-based record index.
///
/// `ctx.selection` is consulted to choose the per-cell background: cells
/// inside the selection rectangle are filled with `theme::BG_SELECTED`.
/// `ctx.copy_flash` is the rectangle of the most recent copy; while set,
/// those cells are filled with `theme::BG_COPIED` instead, which is the
/// short green "copied" confirmation.
///
/// `mouse_down` and `mouse_move` are wired on every visible cell.
/// `mouse_down` receives the row, column, the event (so the shift
/// modifier can be inspected), and a `&mut App` (so the handler can
/// route an `entity.update` back to the `MainView`). `mouse_move`
/// receives the same, but only fires while the pointer is over that
/// cell; the caller checks `MouseMoveEvent::dragging()` to implement
/// drag-select. Because each cell is its own hitbox, drag-select needs
/// no coordinate arithmetic or vertical-scroll bookkeeping - whichever
/// cell the pointer is over reports itself.
pub fn render_row<F, G>(
    model: &GridModel,
    index: u64,
    ctx: &RowContext<'_>,
    mouse_down: F,
    mouse_move: G,
) -> impl IntoElement
where
    F: Fn(&MouseDownEvent, u64, usize, &mut gpui::App) + Clone + 'static,
    G: Fn(&MouseMoveEvent, u64, usize, &mut gpui::App) + Clone + 'static,
{
    let selection = ctx.selection;
    let scroll_x = ctx.scroll_x;
    let viewport_w = ctx.viewport_w;
    let copy_flash = ctx.copy_flash;
    let search_matches = ctx.search_matches;
    let search_active = ctx.search_active;
    let search_query = ctx.search_query;
    let search_options = &ctx.search_options;

    // Row-level search state. Computed once for the row and reused
    // per cell so the binary_search into `search_matches` does not
    // repeat. Substring highlighting fires on any match row, not
    // just the active row, so every matched row's cells get the
    // `find_ranges` scan.
    let row_search_match = search_matches.binary_search(&index).is_ok();
    // The needle as bytes for `find_ranges` is only needed when this
    // is a match row, the needle is non-empty, and we are about to
    // render a cell whose bytes are available.
    let needle_bytes = if row_search_match && !search_query.is_empty() {
        Some(search_query.as_bytes())
    } else {
        None
    };
    // Whether this record carries a parse warning, and which kind.
    // Computed once per row so the gutter can flag it without a
    // per-cell scan.
    let row_warning = warning_kind_for(ctx.warning_rows, index);

    let vis = model.layout.visible(scroll_x, viewport_w);
    let pad = model.layout.offset_of(vis.start);
    let mut cells = Vec::with_capacity(vis.len());
    // Decode the record's visible cells in a single forward pass.
    // Calling `doc.cell` per column re-walked the record from its start
    // each time (O(visible_columns x record length)); one `Cells` pass
    // is O(record length). Cells stay borrowed from the mmap, and only
    // the element's text is copied into a `SharedString`.
    let mut cell_iter = model.doc.record_bytes(index).map(|record| {
        Cells::new(record, model.doc.meta.delimiter, model.doc.meta.quote).skip(vis.start)
    });
    for col in vis.clone() {
        let cow = cell_iter.as_mut().and_then(|iter| iter.next());
        let bytes: &[u8] = match &cow {
            Some(cell) => cell.as_ref(),
            None => &[],
        };
        let is_selected = selection.contains(index, col);
        let is_flashed = rect_contains(copy_flash, index, col);
        let down_handler = mouse_down.clone();
        let move_handler = mouse_move.clone();
        let mut cell_div = div()
            .flex_shrink_0()
            .w(model.layout.width_of(col))
            .px(theme::CELL_PAD_X)
            .truncate();
        if is_flashed {
            cell_div = cell_div.bg(theme::BG_COPIED);
        } else if is_selected {
            cell_div = cell_div.bg(theme::BG_SELECTED);
        }
        if bytes.is_empty() {
            cell_div = cell_div.text_color(theme::TEXT_EMPTY).child("∅");
        } else {
            // Byte-exact text, served from the visible-range cache so a
            // scroll step only allocates for the cells that just came
            // into view. Non-UTF-8 bytes keep their escaped form.
            let text = model.cell_text(index, col, bytes);
            if should_highlight(search_options, col) {
                if let Some(needle) = needle_bytes {
                    // Match row: highlight every occurrence of the needle
                    // inside the cell. `find_ranges` returns byte ranges;
                    // the cost is one case-folded copy per visible cell in
                    // a match row (~30 rows x ~6 columns).
                    let ranges = find_ranges(bytes, needle, search_options.case_sensitive);
                    if ranges.is_empty() {
                        // Row matched but this cell did not: render plain.
                        cell_div = cell_div.child(text);
                    } else {
                        let highlight = HighlightStyle {
                            background_color: Some(Hsla::from(theme::BG_MATCH_TEXT)),
                            ..Default::default()
                        };
                        let styled = StyledText::new(text)
                            .with_highlights(ranges.into_iter().map(|r| (r, highlight)));
                        cell_div = cell_div.child(styled);
                    }
                } else {
                    cell_div = cell_div.child(text);
                }
            } else {
                cell_div = cell_div.child(text);
            }
        }
        // Wire the click on the cell. The captured closure receives
        // the live `MouseDownEvent` (with the shift modifier) plus a
        // `&mut App` so the caller can route an `entity.update` back
        // to the `MainView` and mutate `selection`.
        cell_div = cell_div
            .on_mouse_down(gpui::MouseButton::Left, move |ev, _window, app_cx| {
                down_handler(ev, index, col, app_cx);
            })
            .on_mouse_move(move |ev, _window, app_cx| {
                move_handler(ev, index, col, app_cx);
            });
        cells.push(cell_div);
    }

    // Row-level selection: when the user has selected the entire row we
    // paint a row-wide background so the row number gutter and any
    // column outside the visible range still look selected. This is a
    // cheap, cosmetic touch; the authoritative state is still the
    // `selection.contains` check above.
    let row_selected = selection
        .normalize()
        .map(|(tl, br)| {
            tl.column_index == 0 && br.column_index >= model.layout.widths.len().saturating_sub(1)
        })
        .unwrap_or(false);
    let row_flashed = copy_flash
        .map(|(tl, br)| {
            index >= tl.record_index
                && index <= br.record_index
                && tl.column_index == 0
                && br.column_index >= model.layout.widths.len().saturating_sub(1)
        })
        .unwrap_or(false);

    // Search-match row tint. The active row uses a stronger colour so
    // the user can see which match Enter / Shift+Enter will land on.
    // Reuses the binary-search result computed at the top of the
    // function so we do not scan the match list twice.
    let row_search_active =
        row_search_match && search_active >= 0 && search_matches[search_active as usize] == index;

    let bg = if row_flashed {
        theme::BG_COPIED
    } else if row_search_active {
        theme::BG_MATCH_ACTIVE
    } else if row_search_match {
        theme::BG_MATCH
    } else if row_selected {
        theme::BG_SELECTED
    } else if index % 2 == 1 {
        theme::BG_ROW_ALT
    } else {
        theme::BG
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .h(theme::ROW_HEIGHT)
        .w_full()
        .bg(bg)
        .border_b_1()
        .border_color(rgb(0x2a2a2a))
        .child(
            div()
                .flex_shrink_0()
                .w(theme::ROW_NUMBER_WIDTH)
                .px(theme::CELL_PAD_X)
                .text_color(match row_warning {
                    Some(kind) => warning_color(kind),
                    None => theme::TEXT_DIM,
                })
                .child(match row_warning {
                    Some(kind) => format!("{} {}", warning_glyph(kind), index + 1),
                    None => format!("{}", index + 1),
                }),
        )
        .child(
            div().flex_1().overflow_hidden().child(
                div()
                    .flex()
                    .flex_row()
                    .ml(px(-scroll_x))
                    .child(div().flex_shrink_0().w(pad))
                    .children(cells),
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_core::SearchOptions;

    #[test]
    fn rect_contains_respects_inclusive_bounds() {
        let rect = Some((CellPos::new(1, 1), CellPos::new(3, 4)));
        assert!(rect_contains(rect, 1, 1));
        assert!(rect_contains(rect, 3, 4));
        assert!(rect_contains(rect, 2, 3));
        assert!(!rect_contains(rect, 0, 1));
        assert!(!rect_contains(rect, 1, 0));
        assert!(!rect_contains(rect, 4, 1));
        assert!(!rect_contains(rect, 1, 5));
    }

    #[test]
    fn rect_contains_is_false_when_none() {
        assert!(!rect_contains(None, 0, 0));
    }

    /// The `SharedString` handed to the cell element must carry exactly
    /// the bytes `exact_string` produces: plain ASCII and multi-byte
    /// UTF-8 verbatim, padding and leading zeroes preserved, and the
    /// escaped `\xNN` form for invalid UTF-8. This guards the
    /// one-allocation change against any accidental re-encoding.
    #[test]
    fn cell_shared_string_is_byte_faithful() {
        assert_eq!(cell_shared_string(b"hello").as_ref(), "hello");
        assert_eq!(cell_shared_string(b"").as_ref(), "");
        // Padding and leading zeroes are preserved, not trimmed.
        assert_eq!(cell_shared_string(b" 007 ").as_ref(), " 007 ");
        // Multi-byte UTF-8 survives in one piece.
        let multibyte = "h\u{e9}llo \u{2192} \u{65e5}\u{672c}\u{8a9e}";
        assert_eq!(cell_shared_string(multibyte.as_bytes()).as_ref(), multibyte);
        // A valid cell containing a literal backslash keeps it as-is.
        assert_eq!(cell_shared_string(br"back\slash").as_ref(), "back\\slash");
        // Invalid UTF-8 is escaped exactly like `exact_string`.
        for bytes in [
            &[0xffu8, 0xfe][..],
            &[0x41, 0xff, 0x42],
            &[b'\\', 0xff],
            &[0xe6, 0x97, 0xa5, 0xff],
        ] {
            let expected = exact_string(bytes).into_owned();
            assert_eq!(
                cell_shared_string(bytes).as_ref(),
                expected,
                "escaped form must match exact_string for {bytes:?}"
            );
            // The escaped output must be valid UTF-8 (reversible text).
            assert!(expected.is_ascii() || std::str::from_utf8(expected.as_bytes()).is_ok());
        }
    }

    #[test]
    fn warning_kind_for_uses_sorted_membership_and_severity() {
        let rows = [
            (1u64, WarningKind::InconsistentColumnCount),
            (4, WarningKind::VeryLongRecord),
            (9, WarningKind::UnterminatedQuote),
        ];
        assert_eq!(warning_kind_for(&rows, 0), None);
        assert_eq!(
            warning_kind_for(&rows, 1),
            Some(WarningKind::InconsistentColumnCount)
        );
        assert_eq!(warning_kind_for(&rows, 2), None);
        assert_eq!(
            warning_kind_for(&rows, 4),
            Some(WarningKind::VeryLongRecord)
        );
        assert_eq!(
            warning_kind_for(&rows, 9),
            Some(WarningKind::UnterminatedQuote)
        );
        assert_eq!(warning_kind_for(&rows, 10), None);
        assert_eq!(warning_kind_for(&[], 3), None);
    }

    #[test]
    fn cache_warning_rows_keeps_the_most_severe_kind_per_record() {
        // A record that is both unterminated and very long shows the
        // quote (the root cause); records without an index are dropped.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("warn.csv");
        let big = "x".repeat(2 * 1024 * 1024);
        std::fs::write(&path, format!("a,b\n{big},\"unterminated\n")).expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        let rows = cache_warning_rows(&doc);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 0);
        assert_eq!(rows[0].1, WarningKind::UnterminatedQuote);
    }

    #[test]
    fn should_highlight_honours_column_scope() {
        // No scope: every column in a match row is eligible.
        let all = SearchOptions::default();
        assert!(should_highlight(&all, 0));
        assert!(should_highlight(&all, 3));

        // Scoped: only the selected column highlights.
        let scoped = SearchOptions {
            column: Some(2),
            ..SearchOptions::default()
        };
        assert!(!should_highlight(&scoped, 0));
        assert!(should_highlight(&scoped, 2));
        assert!(!should_highlight(&scoped, 3));
    }

    /// A manual resize clamps to the min/max and recomputes offsets and
    /// the total so header, body and scrollbar stay aligned.
    #[test]
    fn resized_clamps_and_recomputes_offsets() {
        let layout = ColumnLayout::from_widths(vec![100.0, 200.0, 300.0]);
        assert_eq!(layout.offsets, vec![0.0, 100.0, 300.0, 600.0]);
        assert_eq!(layout.total, 600.0);

        // Clamp low and high.
        let shrunk = layout.resized(1, 1.0);
        assert_eq!(shrunk.widths[1], f32::from(theme::MIN_COLUMN_WIDTH));
        let grown = layout.resized(1, 100_000.0);
        let max = f32::from(theme::MAX_COLUMN_WIDTH);
        assert_eq!(grown.widths[1], max);
        assert_eq!(grown.offsets[2], 100.0 + max);
        assert_eq!(grown.total, 100.0 + max + 300.0);

        // An in-range resize is exact and shifts later offsets.
        let normal = layout.resized(0, 150.0);
        assert_eq!(normal.widths, vec![150.0, 200.0, 300.0]);
        assert_eq!(normal.offsets, vec![0.0, 150.0, 350.0, 650.0]);
        assert_eq!(normal.total, 650.0);

        // An out-of-range column leaves the layout unchanged.
        let untouched = layout.resized(9, 500.0);
        assert_eq!(untouched.widths, vec![100.0, 200.0, 300.0]);
    }

    /// The bounded visible-range cache returns identical text on a hit,
    /// keeps the escaped form for invalid bytes, and never grows past
    /// its cap.
    #[test]
    fn cell_text_cache_is_consistent_and_bounded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cache.csv");
        std::fs::write(&path, b"a,b\nalpha,x\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        let model = GridModel::new(std::sync::Arc::new(doc));

        let first = model.cell_text(0, 0, b"alpha");
        let second = model.cell_text(0, 0, b"alpha");
        assert_eq!(first.as_ref(), "alpha");
        assert_eq!(first.as_ref(), second.as_ref());

        // Non-UTF-8 stays in the escaped form the grid shows.
        let expected = exact_string(&[0xff, 0x41]).into_owned();
        assert_eq!(model.cell_text(0, 1, &[0xff, 0x41]).as_ref(), expected);

        // Push well past the cap and confirm it is cleared, not grown.
        for i in 0..(CELL_TEXT_CACHE_CAP as u64 + 128) {
            let value = format!("value-{i}");
            let _ = model.cell_text(i, 0, value.as_bytes());
        }
        let len = model.text_cache.lock().expect("cache lock").len();
        assert!(
            len <= CELL_TEXT_CACHE_CAP,
            "cache grew to {len}, cap is {CELL_TEXT_CACHE_CAP}"
        );
    }

    /// Smoke test: render_row with a populated search query and a
    /// match in the rendered row should not panic and should build
    /// the substring highlight path. We can't observe the rendered
    /// `StyledText` directly from the headless harness, but the
    /// regression we care about (panicking because the byte ranges
    /// don't line up with `SharedString` boundaries, or because
    /// `find_ranges` returned an unexpected value) would surface
    /// here.
    #[test]
    fn render_row_substring_highlight_does_not_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hl.csv");
        std::fs::write(&path, b"a,b\nneedle,other\nfoo,needle bar\nbaz,qux\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        let model = GridModel::new(std::sync::Arc::new(doc));

        let selection = Selection::empty();
        let ctx = RowContext {
            selection: &selection,
            scroll_x: 0.0,
            viewport_w: 1000.0,
            copy_flash: None,
            search_matches: &[0, 1],
            search_active: 1,
            search_query: "needle",
            search_options: SearchOptions::default(),
            warning_rows: &[],
        };

        // Row 0 is a match (it contains "needle" in column 0).
        // Row 1 is the active match (it contains "needle" in column 1,
        // followed by " bar"). Rendering either row must not panic.
        let _elem0 = render_row(&model, 0, &ctx, |_, _, _, _| {}, |_, _, _, _| {});
        let _elem1 = render_row(&model, 1, &ctx, |_, _, _, _| {}, |_, _, _, _| {});
    }

    /// A non-match row should not run the substring scan path.
    /// Render it and rely on the absence of panics as the assertion;
    /// we cannot observe the resulting element from this harness.
    #[test]
    fn render_row_non_match_does_not_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hl.csv");
        std::fs::write(&path, b"a,b\nneedle,other\nfoo,needle bar\nbaz,qux\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        let model = GridModel::new(std::sync::Arc::new(doc));

        let selection = Selection::empty();
        let ctx = RowContext {
            selection: &selection,
            scroll_x: 0.0,
            viewport_w: 1000.0,
            copy_flash: None,
            search_matches: &[0, 1],
            search_active: 1,
            search_query: "needle",
            search_options: SearchOptions::default(),
            warning_rows: &[],
        };

        let _elem = render_row(&model, 2, &ctx, |_, _, _, _| {}, |_, _, _, _| {});
    }

    /// A column-scoped search option must still render a match row
    /// without panicking, rendering non-scoped columns plain while
    /// highlighting only the scoped column.
    #[test]
    fn render_row_with_column_scope_does_not_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scoped.csv");
        std::fs::write(&path, b"a,b\nneedle,other\nfoo,needle bar\nbaz,qux\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        let model = GridModel::new(std::sync::Arc::new(doc));

        let selection = Selection::empty();
        let ctx = RowContext {
            selection: &selection,
            scroll_x: 0.0,
            viewport_w: 1000.0,
            copy_flash: None,
            search_matches: &[0, 1],
            search_active: 1,
            search_query: "needle",
            search_options: SearchOptions {
                column: Some(0),
                ..SearchOptions::default()
            },
            warning_rows: &[],
        };

        // Row 0's match is in the scoped column; row 1's match is in
        // column 1, so its column 1 cell is gated off.
        let _elem0 = render_row(&model, 0, &ctx, |_, _, _, _| {}, |_, _, _, _| {});
        let _elem1 = render_row(&model, 1, &ctx, |_, _, _, _| {}, |_, _, _, _| {});
    }

    /// Layout-throughput benchmark for a very wide file (500 columns x
    /// 300 rows). Run with:
    /// `cargo test --release --bin anvil measure_wide_column_layout -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_wide_column_layout() {
        let cols = 500usize;
        let rows = 300usize;
        let mut csv = String::with_capacity(cols * rows * 4);
        for c in 0..cols {
            if c > 0 {
                csv.push(',');
            }
            csv.push_str(&format!("h{c}"));
        }
        csv.push('\n');
        for r in 0..rows {
            for c in 0..cols {
                if c > 0 {
                    csv.push(',');
                }
                csv.push_str(&format!("{r}-{c}"));
            }
            csv.push('\n');
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("wide.csv");
        std::fs::write(&path, csv.as_bytes()).expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        assert_eq!(doc.column_count(), cols);
        let started = std::time::Instant::now();
        let layout = ColumnLayout::compute(&doc);
        let elapsed = started.elapsed();
        eprintln!(
            "layout {} cols x {} rows in {:.0?}",
            cols,
            doc.record_count(),
            elapsed
        );
        assert_eq!(layout.widths.len(), cols);
    }
}

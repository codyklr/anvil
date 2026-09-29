//! Virtualized grid layout and row rendering.
//!
//! Vertical virtualization is provided by GPUI's `uniform_list`. Horizontal
//! virtualization is done here: `ColumnLayout` knows each column's pixel
//! width and offset, so a row only builds elements for the columns that are
//! actually visible. That keeps a 2,000-column file cheap to display.

use std::ops::Range;
use std::sync::Arc;

use gpui::{div, prelude::*, px, rgb, IntoElement, MouseDownEvent, MouseMoveEvent, Pixels};

use anvil_core::{CellPos, ColumnIndex, ParsedDocument, Selection};

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

        for (col, width) in widths.iter_mut().enumerate().take(measured) {
            let mut widest =
                text_width(doc.meta.headers.get(col).map(String::as_str).unwrap_or(""));
            for row in 0..sampled_rows {
                if let Some(cell) = doc.cell(row, ColumnIndex(col)) {
                    let w = text_width(cell.as_str());
                    if w > widest {
                        widest = w;
                    }
                }
                if widest >= MAX_AUTO_WIDTH {
                    break;
                }
            }
            *width = (widest + pad).clamp(min_w, MAX_AUTO_WIDTH);
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

/// A read-only view over one document that the grid renders.
#[derive(Clone)]
pub struct GridModel {
    pub doc: Arc<ParsedDocument>,
    pub layout: Arc<ColumnLayout>,
}

impl GridModel {
    pub fn new(doc: Arc<ParsedDocument>) -> Self {
        let layout = ColumnLayout::compute(&doc);
        Self {
            doc,
            layout: Arc::new(layout),
        }
    }

    pub fn row_count(&self) -> u64 {
        self.doc.record_count()
    }
}

/// The sticky header row. Renders only the visible column headers.
pub fn render_header(model: &GridModel, scroll_x: f32, viewport_w: f32) -> impl IntoElement {
    let vis = model.layout.visible(scroll_x, viewport_w);
    let pad = model.layout.offset_of(vis.start);
    let mut cells = Vec::with_capacity(vis.len());
    for col in vis {
        let label = model
            .doc
            .meta
            .headers
            .get(col)
            .cloned()
            .unwrap_or_else(|| format!("col {col}"));
        cells.push(
            div()
                .flex_shrink_0()
                .w(model.layout.width_of(col))
                .px(theme::CELL_PAD_X)
                .truncate()
                .text_color(theme::TEXT_STRONG)
                .child(label),
        );
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
/// no coordinate arithmetic or vertical-scroll bookkeeping — whichever
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
    let vis = model.layout.visible(scroll_x, viewport_w);
    let pad = model.layout.offset_of(vis.start);
    let mut cells = Vec::with_capacity(vis.len());
    for col in vis {
        let text = model
            .doc
            .cell(index, ColumnIndex(col))
            .map(|c| c.into_string())
            .unwrap_or_default();
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
        if text.is_empty() {
            cell_div = cell_div.text_color(theme::TEXT_EMPTY).child("∅");
        } else {
            cell_div = cell_div.child(text);
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
    let bg = if row_flashed {
        theme::BG_COPIED
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
                .text_color(theme::TEXT_DIM)
                .child(format!("{}", index + 1)),
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
}

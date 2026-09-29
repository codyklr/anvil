//! Virtualized grid layout and row rendering.
//!
//! Vertical virtualization is provided by GPUI's `uniform_list`. Horizontal
//! virtualization is done here: `ColumnLayout` knows each column's pixel
//! width and offset, so a row only builds elements for the columns that are
//! actually visible. That keeps a 2,000-column file cheap to display.

use std::ops::Range;
use std::sync::Arc;

use gpui::{div, prelude::*, px, rgb, IntoElement, Pixels};

use anvil_core::{ColumnIndex, ParsedDocument};

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

        for col in 0..measured {
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
            widths[col] = (widest + pad).clamp(min_w, MAX_AUTO_WIDTH);
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

/// Renders a single data row. `index` is the 0-based record index.
pub fn render_row(
    model: &GridModel,
    index: u64,
    scroll_x: f32,
    viewport_w: f32,
    selected: bool,
) -> impl IntoElement {
    let vis = model.layout.visible(scroll_x, viewport_w);
    let pad = model.layout.offset_of(vis.start);
    let mut cells = Vec::with_capacity(vis.len());
    for col in vis {
        let text = model
            .doc
            .cell(index, ColumnIndex(col))
            .map(|c| c.into_string())
            .unwrap_or_default();
        if text.is_empty() {
            cells.push(
                div()
                    .flex_shrink_0()
                    .w(model.layout.width_of(col))
                    .px(theme::CELL_PAD_X)
                    .truncate()
                    .text_color(theme::TEXT_EMPTY)
                    .child("∅"),
            );
        } else {
            cells.push(
                div()
                    .flex_shrink_0()
                    .w(model.layout.width_of(col))
                    .px(theme::CELL_PAD_X)
                    .truncate()
                    .child(text),
            );
        }
    }

    let bg = if selected {
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

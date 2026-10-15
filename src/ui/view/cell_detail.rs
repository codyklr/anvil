//! Full-cell detail panel (Cmd/Ctrl+Enter).
//!
//! Reads one page of the cursor cell on the background executor. A page
//! is at most `CELL_DETAIL_CAP` source bytes, so a multi-MB cell cannot
//! block the UI. Previous / Next and Alt+Left / Alt+Right / Alt+Home /
//! Alt+End move through the rest. Copy stays the full exact value. The
//! panel state and `CellDetail` type live on `MainView` in `view.rs`;
//! only the methods are split out here.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable};

use super::*;

impl MainView {
    // ---- full-cell detail panel ------------------------------------------

    /// Toggle the full-cell panel (Cmd/Ctrl+Enter).
    pub(super) fn toggle_cell_detail(&mut self, cx: &mut Context<Self>) {
        if self.cell_detail_open {
            self.close_cell_detail(cx);
        } else {
            self.open_cell_detail(cx);
        }
    }

    pub(super) fn open_cell_detail(&mut self, cx: &mut Context<Self>) {
        if self.model.is_none() {
            return;
        }
        self.inspector_open = false;
        self.diagnostics_open = false;
        self.cell_detail_open = true;
        self.kick_cell_detail(cx);
    }

    pub(super) fn close_cell_detail(&mut self, cx: &mut Context<Self>) {
        self.cell_detail_open = false;
        self.cell_detail = None;
        self.cell_detail_generation = self.cell_detail_generation.wrapping_add(1);
        self.cell_detail_task = None;
        self.cell_detail_offset = 0;
        self.cell_detail_anchor = None;
        cx.notify();
    }

    /// Show another page of the cell the panel is already reading.
    /// A cursor move resets the page in [`Self::kick_cell_detail`].
    pub(super) fn page_cell_detail(&mut self, page: CellDetailPage, cx: &mut Context<Self>) {
        if !self.cell_detail_open {
            return;
        }
        let Some(detail) = self.cell_detail.clone() else {
            if matches!(page, CellDetailPage::First) {
                self.cell_detail_offset = 0;
                self.kick_cell_detail(cx);
            }
            return;
        };
        let total = detail.total_bytes as usize;
        let next = match page {
            CellDetailPage::First => 0,
            CellDetailPage::Last => total.saturating_sub(CELL_DETAIL_CAP),
            CellDetailPage::Next => {
                let end = detail.end as usize;
                if end >= total {
                    return;
                }
                end
            }
            CellDetailPage::Previous => {
                let start = detail.start as usize;
                if start == 0 {
                    return;
                }
                start.saturating_sub(CELL_DETAIL_CAP)
            }
        };
        if next == self.cell_detail_offset && detail.start as usize == next {
            return;
        }
        self.cell_detail_offset = next;
        self.kick_cell_detail(cx);
    }

    /// Read one page of the cursor cell on the background executor.
    /// The read is bounded to `CELL_DETAIL_CAP` bytes so a multi-MB cell
    /// cannot block the UI thread; a newer cursor move or page request
    /// bumps the generation and discards an in-flight read. Changing
    /// cells returns the offset to the first page.
    pub(super) fn kick_cell_detail(&mut self, cx: &mut Context<Self>) {
        if !self.cell_detail_open {
            return;
        }
        let Some(model) = self.model.clone() else {
            return;
        };
        // Fall back to the first cell when nothing is selected yet.
        let pos = self.selection.cursor().unwrap_or(CellPos::new(0, 0));
        // The selection is in display space; resolve the position to the
        // record it points at before reading the cell on the background
        // executor. A stale position past the view reads record 0.
        let record = match self.view_rows() {
            Some(rows) => rows.get(pos.row as usize).copied().unwrap_or(0),
            None => pos.row,
        };
        let column = pos.column_index;
        let anchor = (record, column);
        if self.cell_detail_anchor != Some(anchor) {
            self.cell_detail_offset = 0;
            self.cell_detail_anchor = Some(anchor);
        }
        let offset = self.cell_detail_offset;
        let encoding = self.display_encoding;
        self.cell_detail_generation = self.cell_detail_generation.wrapping_add(1);
        let generation = self.cell_detail_generation;
        let doc = model.doc.clone();
        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let detail = executor
                .spawn(async move {
                    let cell = doc.cell(record, ColumnIndex(column));
                    let bytes: &[u8] = match &cell {
                        Some(cell) => cell.as_bytes(),
                        None => &[],
                    };
                    let window =
                        anvil_core::display_window(bytes, offset, CELL_DETAIL_CAP, encoding);
                    CellDetail {
                        record,
                        column,
                        label: doc.meta.column_label(column),
                        text: window.text.into_owned(),
                        total_bytes: bytes.len() as u64,
                        start: window.start as u64,
                        end: window.end as u64,
                        truncated: window.more_before || window.more_after,
                        adjusted_boundary: window.adjusted_boundary,
                    }
                })
                .await;
            this.update(cx, |view, cx| {
                if view.cell_detail_generation != generation || !view.cell_detail_open {
                    return;
                }
                view.cell_detail = Some(Arc::new(detail));
                view.cell_detail_task = None;
                cx.notify();
            })
            .ok();
        });
        self.cell_detail_task = Some(task);
        cx.notify();
    }

    /// The full-cell panel: the cursor cell's exact value, capped and
    /// labelled when the cap is hit.
    pub(super) fn render_cell_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme::palette(cx);
        let header = panel::header(
            colors,
            IconName::FileText,
            "Cell",
            Button::new("cell-detail-close")
                .xsmall()
                .ghost()
                .icon(IconName::Close)
                .accessibility_label("Close cell")
                .tooltip("Close cell")
                .on_click(cx.listener(|this, _, _, cx| this.close_cell_detail(cx))),
        );
        let content: Vec<AnyElement> = match &self.cell_detail {
            Some(detail) => {
                let mut out = vec![
                    panel::title_block(
                        cx,
                        colors,
                        detail.label.clone(),
                        format!(
                            "Row {} · Column {}",
                            format_count(detail.record + 1),
                            format_count((detail.column + 1) as u64)
                        ),
                    )
                    .into_any_element(),
                    panel::caption_owned(
                        colors,
                        cell_detail_caption(detail, self.display_encoding),
                    )
                    .into_any_element(),
                ];
                if detail.truncated {
                    let at_start = detail.start == 0;
                    let at_end = detail.end >= detail.total_bytes;
                    out.push(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px(px(panel::BODY_PAD_X))
                            .pt(px(4.))
                            .child(
                                Button::new("cell-detail-previous")
                                    .xsmall()
                                    .ghost()
                                    .label("Previous")
                                    .disabled(at_start)
                                    .accessibility_label("Previous page")
                                    .tooltip("Previous page · Alt+Left")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.page_cell_detail(CellDetailPage::Previous, cx)
                                    })),
                            )
                            .child(
                                Button::new("cell-detail-next")
                                    .xsmall()
                                    .ghost()
                                    .label("Next")
                                    .disabled(at_end)
                                    .accessibility_label("Next page")
                                    .tooltip("Next page · Alt+Right")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.page_cell_detail(CellDetailPage::Next, cx)
                                    })),
                            )
                            .into_any_element(),
                    );
                }
                // The value itself: the whole point of the panel, so it gets
                // the surface, the monospace face and the room.
                out.push(
                    div()
                        .id("cell-detail-body")
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .mx(px(panel::BODY_PAD_X))
                        .overflow_y_scroll()
                        .rounded(cx.theme().radius_tokens().sm)
                        .bg(theme::blend(colors.panel, colors.canvas, 0.6))
                        .border_1()
                        .border_color(colors.border)
                        .child(
                            div()
                                .p(px(8.))
                                .min_w_0()
                                .whitespace_normal()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_xs()
                                .line_height(px(17.))
                                .text_color(if detail.text.is_empty() {
                                    colors.empty_foreground
                                } else {
                                    colors.foreground
                                })
                                .child(if detail.text.is_empty() {
                                    "Empty value, 0 bytes".to_string()
                                } else {
                                    detail.text.clone()
                                }),
                        )
                        .into_any_element(),
                );
                out
            }
            None => vec![panel::empty_state(
                colors,
                if self.model.is_some() {
                    "Reading cell"
                } else {
                    "No document open"
                },
                if self.model.is_some() {
                    "The exact value of the cursor cell appears here."
                } else {
                    "Open a data file to inspect a cell's value."
                },
            )],
        };
        div()
            .w_full()
            .min_w_0()
            .h_full()
            .font_family(cx.theme().font_family.clone())
            .text_sm()
            .flex()
            .flex_col()
            .bg(colors.panel)
            .child(header)
            .child(panel::body("cell-detail-scroll", content))
            .into_any_element()
    }
}

/// Caption for the page on screen. A cell that fits in one page keeps
/// the old wording. A longer cell names the source bytes actually shown.
fn cell_detail_caption(detail: &CellDetail, encoding: anvil_core::DisplayEncoding) -> String {
    let mut text = if !detail.truncated {
        format!("{} bytes, exact value", format_count(detail.total_bytes))
    } else {
        let mut text = format!(
            "Bytes {}-{} of {}",
            format_count(detail.start + 1),
            format_count(detail.end),
            format_count(detail.total_bytes)
        );
        if detail.adjusted_boundary {
            text.push_str(". Cut moved to a character boundary");
        }
        text
    };
    if encoding != anvil_core::DisplayEncoding::Utf8 {
        text.push_str(", ");
        text.push_str(encoding.label());
    }
    text
}

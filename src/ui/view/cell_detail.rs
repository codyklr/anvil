//! Full-cell detail panel (Cmd/Ctrl+Enter).
//!
//! Reads the cursor cell on the background executor and shows its exact
//! value, capped so a multi-MB cell cannot block the UI. The panel state
//! and `CellDetail` type live on `MainView` in `view.rs`; only the
//! methods are split out here.

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
        self.cell_detail_open = true;
        self.kick_cell_detail(cx);
    }

    pub(super) fn close_cell_detail(&mut self, cx: &mut Context<Self>) {
        self.cell_detail_open = false;
        self.cell_detail = None;
        self.cell_detail_generation = self.cell_detail_generation.wrapping_add(1);
        self.cell_detail_task = None;
        cx.notify();
    }

    /// Read the cursor cell's exact value on the background executor.
    /// The read is bounded to `CELL_DETAIL_CAP` bytes so a multi-MB cell
    /// cannot block the UI thread; a newer cursor move bumps the
    /// generation and discards an in-flight read.
    pub(super) fn kick_cell_detail(&mut self, cx: &mut Context<Self>) {
        if !self.cell_detail_open {
            return;
        }
        let Some(model) = self.model.clone() else {
            return;
        };
        // Fall back to the first cell when nothing is selected yet.
        let pos = self.selection.cursor().unwrap_or(CellPos::new(0, 0));
        self.cell_detail_generation = self.cell_detail_generation.wrapping_add(1);
        let generation = self.cell_detail_generation;
        let doc = model.doc.clone();
        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let detail = executor
                .spawn(async move {
                    let cell = doc.cell(pos.record_index, ColumnIndex(pos.column_index));
                    let bytes: &[u8] = match &cell {
                        Some(cell) => cell.as_bytes(),
                        None => &[],
                    };
                    let (text, truncated) = anvil_core::exact_string_capped(bytes, CELL_DETAIL_CAP);
                    CellDetail {
                        record: pos.record_index,
                        column: pos.column_index,
                        label: doc.meta.column_label(pos.column_index),
                        text: text.into_owned(),
                        total_bytes: bytes.len() as u64,
                        truncated,
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
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(div().text_color(theme::TEXT_STRONG).child("Cell"))
            .child(
                div()
                    .id("cell-detail-close")
                    .cursor_pointer()
                    .px_1()
                    .text_color(theme::TEXT_DIM)
                    .hover(|s| s.text_color(theme::WARN))
                    .on_click(cx.listener(|this, _, _, cx| this.close_cell_detail(cx)))
                    .child("×"),
            );
        let body: Vec<AnyElement> = match &self.cell_detail {
            Some(detail) => vec![
                div()
                    .text_color(theme::TEXT_STRONG)
                    .child(format!(
                        "{} (row {}, col {})",
                        detail.label,
                        detail.record + 1,
                        detail.column + 1
                    ))
                    .into_any_element(),
                div()
                    .text_xs()
                    .text_color(theme::TEXT_DIM)
                    .child(if detail.truncated {
                        format!(
                            "showing first {} of {} (capped)",
                            archive_picker::format_size(CELL_DETAIL_CAP as u64),
                            archive_picker::format_size(detail.total_bytes)
                        )
                    } else {
                        archive_picker::format_size(detail.total_bytes)
                    })
                    .into_any_element(),
                div()
                    .id("cell-detail-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(div().text_color(theme::TEXT).child(detail.text.clone()))
                    .into_any_element(),
            ],
            None => vec![div()
                .text_color(theme::TEXT_DIM)
                .child("Reading cell…")
                .into_any_element()],
        };
        div()
            .flex_shrink_0()
            .w(theme::CELL_DETAIL_WIDTH)
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(theme::BG_PANEL)
            .border_l_1()
            .border_color(theme::BORDER)
            .child(header)
            .children(body)
            .into_any_element()
    }
}

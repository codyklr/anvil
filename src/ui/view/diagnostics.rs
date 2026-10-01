//! Diagnostics panel: the parse warnings already collected on the
//! active document, plus a UTF-8 encoding scan, with
//! click-to-navigate to the offending row.
//!
//! `ParseMeta.warnings` is computed once at open and is immutable for
//! the life of the document, so the warnings list is a pure render. The
//! encoding scan is O(file length) and therefore runs on the background
//! executor when the panel opens, guarded by `encoding_generation`, and
//! is cached until the document changes.

use anvil_core::WarningKind;

use super::*;

impl MainView {
    /// Right-side panel listing the active document's parse warnings. A
    /// row warning is clickable and jumps the grid to that record; a
    /// header-level warning (`record_index == None`) is informational
    /// only and not clickable.
    pub(super) fn render_diagnostics(&self, cx: &mut Context<Self>) -> AnyElement {
        let warnings = self
            .model
            .as_ref()
            .map(|m| m.doc.meta.warnings.as_slice())
            .unwrap_or(&[]);

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_3()
            .h(theme::HEADER_HEIGHT)
            .border_b_1()
            .border_color(theme::BORDER)
            .text_color(theme::TEXT_STRONG)
            .child("Diagnostics")
            .child(
                div()
                    .id("diagnostics-close")
                    .cursor_pointer()
                    .text_color(theme::TEXT_DIM)
                    .hover(|s| s.text_color(theme::WARN))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_diagnostics(cx)))
                    .child("×"),
            );

        let warnings_body: AnyElement = if warnings.is_empty() {
            div()
                .p_3()
                .text_color(theme::TEXT_DIM)
                .child("no parse warnings")
                .into_any_element()
        } else {
            let mut rows: Vec<AnyElement> = Vec::with_capacity(warnings.len());
            for (index, warning) in warnings.iter().enumerate() {
                let navigable = warning.record_index.is_some();
                let index_label = match warning.record_index {
                    Some(record) => format!("row {}", record + 1),
                    None => "header".to_string(),
                };
                let mut row = div()
                    .id(("diagnostic-row", index))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(theme::BORDER)
                    .when(navigable, |this| {
                        this.cursor_pointer().hover(|s| s.bg(theme::BG_ROW_HOVER))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::WARN)
                                    .child(warning_kind_label(warning.kind)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_DIM)
                                    .child(index_label),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if navigable {
                                theme::TEXT
                            } else {
                                theme::TEXT_DIM
                            })
                            .child(warning.note.clone()),
                    );
                if navigable {
                    row = row.on_click(
                        cx.listener(move |view, _, _, cx| view.jump_to_warning(index, cx)),
                    );
                }
                rows.push(row.into_any_element());
            }
            div().flex().flex_col().children(rows).into_any_element()
        };

        let body: AnyElement = div()
            .flex()
            .flex_col()
            .child(section_heading("parse warnings"))
            .child(warnings_body)
            .child(section_heading("encoding"))
            .child(self.render_encoding_section())
            .into_any_element();

        div()
            .flex_shrink_0()
            .w(theme::DIAGNOSTICS_WIDTH)
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::BG_PANEL)
            .border_l_1()
            .border_color(theme::BORDER)
            .child(header)
            .child(
                div()
                    .id("diagnostics-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }

    /// The encoding block of the diagnostics panel: a BOM notice and one
    /// row per column with non-UTF-8 cells, or a clean/scanning status.
    /// These findings are column-level, so unlike parse warnings they
    /// have no row to navigate to.
    fn render_encoding_section(&self) -> AnyElement {
        let Some(report) = self.encoding_report.as_ref() else {
            let text = if self.encoding_task.is_some() {
                "scanning…"
            } else {
                "not scanned"
            };
            return div()
                .px_3()
                .py_2()
                .text_xs()
                .text_color(theme::TEXT_DIM)
                .child(text)
                .into_any_element();
        };

        let mut rows: Vec<AnyElement> = Vec::new();
        if report.bom {
            rows.push(encoding_row(
                "UTF-8 BOM",
                "first header cell includes the BOM",
            ));
        }
        for column in &report.columns {
            rows.push(encoding_row(
                &column.name,
                &format!("{} non-UTF-8 cell(s)", column.invalid_cells),
            ));
        }
        if rows.is_empty() {
            rows.push(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(theme::TEXT_DIM)
                    .child("clean UTF-8")
                    .into_any_element(),
            );
        }
        div().flex().flex_col().children(rows).into_any_element()
    }

    /// Move the selection to the warning's row and scroll it into view.
    ///
    /// Warnings are recorded against record indexes, but the grid may be
    /// showing a filtered/sorted view. A record reordered by a sort is
    /// fine to navigate to (the display position is mapped); a record
    /// hidden by a filter is not, because silently scrolling to position
    /// zero would point at an unrelated row. In that case we report the
    /// situation in the status bar instead of moving the cursor.
    pub(super) fn jump_to_warning(&mut self, warning_index: usize, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        let Some(record) = model
            .doc
            .meta
            .warnings
            .get(warning_index)
            .and_then(|w| w.record_index)
        else {
            self.status_message = Some("header-level warning: no row to jump to".to_string());
            cx.notify();
            return;
        };
        let row_count = model.row_count();
        if record >= row_count {
            return;
        }
        if let Some(rows) = self.view_rows() {
            if !rows.contains(&record) {
                self.status_message =
                    Some(format!("row {} is hidden by the active filter", record + 1));
                cx.notify();
                return;
            }
        }
        // Keep the cursor's column so navigation feels like the arrow
        // keys; warnings carry no column of their own.
        let col = self.selection.cursor().map(|c| c.column_index).unwrap_or(0);
        self.selection.set(CellPos::new(record, col));
        let position = self.display_position(record);
        self.v_scroll
            .scroll_to_item_strict(position, ScrollStrategy::Top);
        self.ensure_column_visible(col, cx);
        self.status_message = None;
        cx.notify();
    }
}

/// A small section heading inside the diagnostics panel.
fn section_heading(title: &str) -> AnyElement {
    div()
        .px_3()
        .py_1()
        .text_xs()
        .text_color(theme::TEXT_STRONG)
        .bg(theme::BG_TOOLBAR)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(title.to_string())
        .into_any_element()
}

/// One encoding finding: the column/BOM name on the left, the detail on
/// the right. Encoding findings are column-level and not navigable.
fn encoding_row(name: &str, detail: &str) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(theme::BORDER)
        .child(
            div()
                .flex_1()
                .truncate()
                .text_xs()
                .text_color(theme::WARN)
                .child(name.to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme::TEXT_DIM)
                .child(detail.to_string()),
        )
        .into_any_element()
}

/// Short human label for a warning kind, as shown in the panel.
fn warning_kind_label(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::InconsistentColumnCount => "row width",
        WarningKind::EmptyHeader => "empty header",
        WarningKind::DuplicateHeader => "duplicate header",
        WarningKind::UnterminatedQuote => "unterminated quote",
        WarningKind::VeryLongRecord => "very long record",
    }
}

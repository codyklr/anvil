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

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, IconName, Selectable, Sizable};

use super::*;

impl MainView {
    /// Right-side panel listing the active document's parse warnings. A
    /// row warning is clickable and jumps the grid to that record; a
    /// header-level warning (`record_index == None`) is informational
    /// only and not clickable.
    pub(super) fn render_diagnostics(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme::palette(cx);
        let warnings = self
            .model
            .as_ref()
            .map(|m| m.doc.meta.warnings.as_slice())
            .unwrap_or(&[]);

        let header = panel::header(
            colors,
            IconName::TriangleAlert,
            "Diagnostics",
            Button::new("diagnostics-close")
                .xsmall()
                .ghost()
                .icon(IconName::Close)
                .accessibility_label("Close diagnostics")
                .tooltip("Close diagnostics")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_diagnostics(cx))),
        );

        let mut content: Vec<AnyElement> = vec![panel::title_block(
            cx,
            colors,
            "Parse warnings".to_string(),
            format!(
                "{} warning{} in {}",
                format_count(warnings.len() as u64),
                plural(warnings.len() as u64),
                self.model
                    .as_ref()
                    .map(|m| format_count(m.row_count()))
                    .unwrap_or_else(|| "0 rows".into())
            ),
        )
        .into_any_element()];

        if warnings.is_empty() {
            content.push(panel::caption(colors, "None found").into_any_element());
        } else {
            for (index, warning) in warnings.iter().enumerate() {
                content.push(self.warning_row(cx, colors, warning, index));
            }
        }

        content.push(panel::section(colors, "Encoding").into_any_element());
        content.extend(self.encoding_content(cx));

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
            .child(panel::body("diagnostics-body", content))
            .into_any_element()
    }

    /// One parse warning. Navigable rows are one clickable unit: the whole
    /// row is the hit area, and the row number doubles as the affordance.
    fn warning_row(
        &self,
        cx: &mut Context<Self>,
        colors: theme::Palette,
        warning: &anvil_core::ParseWarning,
        index: usize,
    ) -> AnyElement {
        let navigable = warning.record_index.is_some();
        let location = match warning.record_index {
            Some(record) => format!("Row {}", format_count(record + 1)),
            None => "Header".to_string(),
        };
        div()
            .id(("diagnostic-row", index))
            .flex()
            .flex_col()
            .gap_1()
            .mx(px(panel::BODY_PAD_X))
            .my(px(1.))
            .px(px(8.))
            .py(px(6.))
            .rounded(cx.theme().radius_tokens().sm)
            .bg(if navigable {
                theme::blend(colors.panel, colors.canvas, 0.6)
            } else {
                colors.panel
            })
            .border_1()
            .border_color(if navigable {
                colors.border
            } else {
                colors.panel
            })
            .cursor(if navigable {
                gpui_kit::CursorStyle::PointingHand
            } else {
                gpui_kit::CursorStyle::Arrow
            })
            .when(navigable, |row| {
                row.hover(|style| style.bg(theme::blend(colors.panel, colors.foreground, 0.06)))
            })
            .on_click(cx.listener(move |view, _, _, cx| view.jump_to_warning(index, cx)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(colors.foreground)
                            .child(warning_kind_label(warning.kind)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .font_family(panel::mono(cx))
                            .text_color(if navigable {
                                colors.primary
                            } else {
                                colors.faint_foreground
                            })
                            .child(location),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(warning.note.clone()),
            )
            .into_any_element()
    }

    /// The encoding block of the diagnostics panel: a BOM notice and one
    /// row per column with non-UTF-8 cells, or a clean/scanning status.
    /// These findings are column-level, so unlike parse warnings they
    /// have no row to navigate to.
    fn encoding_content(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = theme::palette(cx);
        let Some(report) = self.encoding_report.as_ref() else {
            let text = if self.encoding_task.is_some() {
                "Scanning for non-UTF-8 values..."
            } else if self.model.is_some() {
                "Encoding has not been scanned"
            } else {
                "No document to scan"
            };
            return vec![
                self.display_encoding_controls(cx),
                panel::caption(colors, text).into_any_element(),
            ];
        };

        let mut rows: Vec<AnyElement> = vec![self.display_encoding_controls(cx)];
        if report.has_invalid_utf8() {
            let shown = match self.display_encoding {
                anvil_core::DisplayEncoding::Utf8 => "shown as reversible escapes",
                anvil_core::DisplayEncoding::Windows1252 => "shown as windows-1252",
            };
            rows.push(
                panel::caption_owned(
                    colors,
                    format!(
                        "{} invalid cells, {shown}",
                        format_count(report.total_invalid_cells())
                    ),
                )
                .into_any_element(),
            );
        }
        if report.bom {
            rows.push(
                panel::metric(cx, colors, "UTF-8 BOM", "present in first header")
                    .into_any_element(),
            );
        }
        for column in &report.columns {
            rows.push(
                panel::metric(
                    cx,
                    colors,
                    &column.name,
                    &format!("{} invalid", format_count(column.invalid_cells)),
                )
                .into_any_element(),
            );
        }
        if !report.has_invalid_utf8() && !report.bom {
            rows.push(
                panel::caption(colors, "All scanned values are valid UTF-8").into_any_element(),
            );
        }
        rows
    }

    /// UTF-8 is the file view. Windows-1252 rereads the same bytes.
    fn display_encoding_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme::palette(cx);
        let current = self.display_encoding;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px(px(panel::BODY_PAD_X))
            .py(px(4.))
            .child(encoding_choice(
                cx,
                current,
                anvil_core::DisplayEncoding::Utf8,
                "display-encoding-utf-8",
                "UTF-8",
            ))
            .child(encoding_choice(
                cx,
                current,
                anvil_core::DisplayEncoding::Windows1252,
                "display-encoding-windows-1252",
                "windows-1252",
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(colors.faint_foreground)
                    .child("view of the file bytes"),
            )
            .into_any_element()
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
        // keys; warnings carry no column of their own. The selection is
        // in display space, so land on the warning record's position.
        let col = self.selection.cursor().map(|c| c.column_index).unwrap_or(0);
        let position = self.display_position(record);
        self.selection.set(CellPos::new(position as u64, col));
        self.v_scroll
            .scroll_to_item_strict(position, ScrollStrategy::Top);
        self.ensure_column_visible(col, cx);
        self.status_message = None;
        cx.notify();
    }
}

fn encoding_choice(
    cx: &mut Context<MainView>,
    current: anvil_core::DisplayEncoding,
    encoding: anvil_core::DisplayEncoding,
    id: &'static str,
    label: &'static str,
) -> gpui_kit::component::button::Button {
    let selected = current == encoding;
    Button::new(id)
        .xsmall()
        .ghost()
        .label(label)
        .selected(selected)
        .toggled(selected)
        .on_click(cx.listener(move |view, _, _, cx| view.apply_display_encoding(encoding, cx)))
}

/// Short human label for a warning kind, as shown in the panel.
fn warning_kind_label(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::InconsistentColumnCount => "Row width",
        WarningKind::EmptyHeader => "Empty header",
        WarningKind::DuplicateHeader => "Duplicate header",
        WarningKind::UnterminatedQuote => "Unterminated quote",
        WarningKind::VeryLongRecord => "Very long record",
    }
}

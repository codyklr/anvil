//! Left-hand file-manager panel rendering.
//!
//! Extracted verbatim from `view.rs` (behaviour-neutral split).

use super::*;

impl MainView {
    /// Left-hand file-manager panel. Lists every open document, with
    /// ZIP entries nested under their archive. Click a row to switch
    /// to it; click × to close it. Toggled with Cmd/Ctrl+B or the
    /// toolbar button; hidden by default when no documents are open.
    pub(super) fn render_file_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = group_documents(&self.documents);
        let mut rows: Vec<AnyElement> = Vec::with_capacity(self.documents.len() + groups.len());
        for (archive, entries) in &groups {
            if let Some(archive) = archive {
                rows.push(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(theme::TEXT_DIM)
                        .child("▾")
                        .child(div().truncate().child(archive.archive_name().to_string()))
                        .into_any_element(),
                );
            }
            for (index, label, loaded) in entries {
                let is_active = Some(*index) == self.active;
                let indented = archive.is_some();
                let select_index = *index;
                let close_index = *index;
                let label_color = if is_active {
                    theme::TEXT_STRONG
                } else {
                    theme::TEXT
                };
                let dot_color = if *loaded { theme::ACCENT } else { theme::WARN };
                let mut row = div()
                    .id(("file-row", *index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .pr_2()
                    .py_1()
                    .cursor_pointer()
                    .when(is_active, |s| s.bg(theme::BG_SELECTED))
                    .hover(|s| s.bg(theme::BG_ROW_HOVER))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.switch_to(select_index, cx);
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(dot_color)
                            .child("●"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_color(label_color)
                            .child(label.clone()),
                    )
                    .child(
                        div()
                            .id(("file-row-close", *index))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::TEXT_STRONG))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |view, _, _, cx| {
                                    view.close_active_at(close_index, cx);
                                }),
                            )
                            .child("×"),
                    );
                row = if indented {
                    row.pl(px(f32::from(theme::FILE_PANEL_ENTRY_INDENT)))
                } else {
                    row.pl_2()
                };
                rows.push(row.into_any_element());
            }
        }

        div()
            .flex()
            .flex_col()
            .w(theme::FILE_PANEL_WIDTH)
            .h_full()
            .flex_shrink_0()
            .bg(theme::BG_PANEL)
            .border_r_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(f32::from(theme::TAB_BAR_HEIGHT)))
                    .px_2()
                    .border_b_1()
                    .border_color(theme::BORDER)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .child(format!("{} open", self.documents.len())),
                    )
                    .child(
                        div()
                            .id("file-panel-close")
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::TEXT_STRONG))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|view, _, _, cx| {
                                    view.file_panel_open = false;
                                    cx.notify();
                                }),
                            )
                            .child("×"),
                    ),
            )
            .child(
                div()
                    .id("file-panel-scroll")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .children(rows),
            )
    }
}

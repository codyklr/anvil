//! Open-document sidebar with archive grouping and explicit close controls.
//!
//! The panel is only useful if it says something the tab strip does not, so
//! each row carries the document's shape (rows x columns, source size) as
//! secondary text. With a single document that turns an otherwise empty
//! column into the fastest read on what is actually loaded.

use super::*;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};

impl MainView {
    /// Lists every open document; ZIP entries remain grouped by archive
    /// identity. Selection and close controls share the same lifecycle as
    /// the tab strip.
    pub(super) fn render_file_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme::palette(cx);
        let groups = group_documents(&self.documents);
        let mut rows: Vec<AnyElement> = Vec::with_capacity(self.documents.len() + groups.len());
        for (archive, entries) in &groups {
            if let Some(archive) = archive {
                rows.push(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px(px(10.))
                        .pt(px(12.))
                        .pb(px(4.))
                        .child(
                            Icon::new(IconName::FolderOpen)
                                .small()
                                .text_color(colors.faint_foreground),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(colors.muted_foreground)
                                .child(archive.archive_name().to_string()),
                        )
                        .into_any_element(),
                );
            }
            for (index, _, _loaded) in entries {
                let snap = &self.documents[*index];
                let id = snap.id;
                let active = Some(*index) == self.active;
                let background = if active {
                    colors.chrome_active
                } else {
                    colors.panel
                };
                let foreground = if active {
                    colors.strong_foreground
                } else {
                    colors.foreground
                };
                let style = ButtonCustomVariant::new(cx)
                    .color(background)
                    .foreground(foreground)
                    .hover(if active {
                        colors.chrome_active
                    } else {
                        colors.hover
                    })
                    .active(colors.chrome_active);
                let label = snap.navigation_label();
                let tooltip = snap.navigation_tooltip();
                // A loaded document reports its shape; a pending one says so
                // instead of showing a dash where a number should be.
                let detail = match &snap.doc {
                    Some(doc) => format!(
                        "{} x {} · {}",
                        format_count(doc.record_count()),
                        format_count(doc.column_count() as u64),
                        archive_picker::format_size(doc.opened.len())
                    ),
                    None if snap.loading => "Opening...".to_string(),
                    None => "Opens on selection".to_string(),
                };

                rows.push(
                    div()
                        .id(("file-row-surface", id))
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded(cx.theme().radius_tokens().sm)
                        .bg(background)
                        .pr(px(2.))
                        .when(archive.is_some(), |row| row.ml(px(10.)))
                        .child(
                            Button::new(("file-row", id))
                                .small()
                                .custom(style)
                                .toggled(active)
                                .flex_1()
                                .min_w_0()
                                .justify_start()
                                .accessibility_label(tooltip.clone())
                                .tooltip(tooltip)
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_2()
                                        .child(snap.navigation_icon(colors))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .flex()
                                                .flex_col()
                                                .items_start()
                                                .gap(px(1.))
                                                .child(
                                                    div()
                                                        .min_w_0()
                                                        .w_full()
                                                        .truncate()
                                                        .font_weight(if active {
                                                            gpui_kit::FontWeight::MEDIUM
                                                        } else {
                                                            gpui_kit::FontWeight::NORMAL
                                                        })
                                                        .child(label.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .min_w_0()
                                                        .w_full()
                                                        .truncate()
                                                        .text_color(colors.faint_foreground)
                                                        .child(detail.clone()),
                                                ),
                                        ),
                                )
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.select_navigation_document(id, cx);
                                })),
                        )
                        .child(
                            Button::new(("file-row-close", id))
                                .xsmall()
                                .ghost()
                                .icon(IconName::Close)
                                .accessibility_label(format!("Close {label}"))
                                .tooltip(format!("Close {label}"))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    cx.stop_propagation();
                                    view.close_navigation_document(id, cx);
                                })),
                        )
                        .into_any_element(),
                );
            }
        }

        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .bg(colors.panel)
            .border_r_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(30.))
                    .pl(px(10.))
                    .pr(px(6.))
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(colors.muted_foreground)
                            .child("OPEN DOCUMENTS"),
                    )
                    .child(
                        Button::new("file-panel-close")
                            .xsmall()
                            .ghost()
                            .icon(IconName::PanelLeftClose)
                            .accessibility_label("Hide files")
                            .tooltip("Hide files · Ctrl/Cmd+B")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.file_panel_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("file-panel-scroll")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(div().flex().flex_col().gap(px(1.)).p(px(6.)).children(rows)),
            )
    }
}

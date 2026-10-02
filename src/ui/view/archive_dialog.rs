//! Kit-owned archive dialog: focus trapping and dismissal belong to the window root.

use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    ActiveTheme, Sizable, WindowExt,
};

impl MainView {
    pub(super) fn sync_archive_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.show_picker || self.archive_dialog_open {
            return;
        }
        let Some(archive) = self.archive.clone() else {
            return;
        };
        self.archive_dialog_open = true;
        let owner = cx.entity().downgrade();
        cx.defer_in(window, move |_, window, cx| {
            window.open_dialog(cx, move |dialog, window, cx| {
                let colors = theme::palette(cx);
                let close_owner = owner.clone();
                let mut rows = Vec::new();
                for candidate in archive.candidates.iter().filter(|entry| entry.usable) {
                    let name = candidate.entry_name.clone();
                    let select_owner = owner.clone();
                    let select_name = name.clone();
                    rows.push(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "archive-entry:{name}"
                                )))
                                .ghost()
                                .small()
                                .icon(IconName::FileText)
                                .label(name)
                                .flex_1()
                                .min_w_0()
                                .on_click(
                                    move |_, window, cx| {
                                        let _ = select_owner.update(cx, |view, cx| {
                                            view.show_picker = false;
                                            view.archive_dialog_open = false;
                                            view.switch_to_entry(&select_name, cx);
                                        });
                                        window.close_dialog(cx);
                                    },
                                ),
                            )
                            .child(
                                div().text_xs().text_color(colors.muted_foreground).child(
                                    archive_picker::format_size(candidate.uncompressed_size),
                                ),
                            ),
                    );
                }
                let skipped = archive_picker::skipped_entries(&archive.candidates);
                let mut content = div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .text_sm()
                    .child(
                        div()
                            .text_color(colors.muted_foreground)
                            .child("Every usable file has a tab. Select a file to inspect it."),
                    )
                    .child(
                        div()
                            .id("archive-files")
                            .flex()
                            .flex_col()
                            .gap_1()
                            .max_h_64()
                            .overflow_y_scroll()
                            .children(rows),
                    );
                if !skipped.is_empty() {
                    content = content
                        .child(
                            div()
                                .pt_3()
                                .border_t_1()
                                .border_color(colors.border)
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(format!("{} skipped entries", skipped.len())),
                        )
                        .child(
                            div()
                                .id("archive-skipped")
                                .max_h_32()
                                .overflow_y_scroll()
                                .children(skipped.iter().take(40).map(|entry| {
                                    div()
                                        .flex()
                                        .gap_3()
                                        .py_1()
                                        .text_xs()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .child(entry.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_color(colors.warning)
                                                .child(entry.reason.clone()),
                                        )
                                })),
                        );
                    if skipped.len() > 40 {
                        content = content.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{} additional skipped entries",
                                    skipped.len() - 40
                                )),
                        );
                    }
                }
                dialog
                    .title(archive.archive_name().to_owned())
                    .width(gpui_kit::rems(36.).to_pixels(window.rem_size()))
                    .child(content)
                    .on_close(move |_, _, cx| {
                        let _ = close_owner.update(cx, |view, cx| {
                            view.show_picker = false;
                            view.archive_dialog_open = false;
                            cx.notify();
                        });
                    })
            });
        });
    }
}

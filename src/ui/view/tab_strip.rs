//! Compact, keyboard-reachable document navigation.

use super::*;
use gpui_kit::component::{
    button::{Button, ButtonCustomVariant, ButtonVariants},
    spinner::Spinner,
    ActiveTheme, Icon, IconName, Sizable,
};

impl DocumentSnapshot {
    /// Unlike `tab_label`, this retains the full name for overflow tooltips.
    pub(super) fn navigation_label(&self) -> String {
        self.entry_name.clone().unwrap_or_else(|| {
            self.doc
                .as_ref()
                .map(|doc| doc.opened.display_label())
                .unwrap_or_else(|| "Loading…".to_string())
        })
    }

    pub(super) fn navigation_tooltip(&self) -> String {
        let label = self.navigation_label();
        let source = match &self.archive {
            Some(archive) => format!("{} / {label}", archive.archive_name()),
            None => label,
        };
        if self.loading {
            format!("{source}: Loading…")
        } else if !self.is_loaded() {
            format!("{source}: Opens on selection")
        } else {
            source
        }
    }

    pub(super) fn navigation_icon(&self, colors: theme::Palette) -> AnyElement {
        if self.loading {
            Spinner::new()
                .small()
                .color(colors.muted_foreground)
                .into_any_element()
        } else {
            Icon::new(if self.is_loaded() {
                IconName::FileText
            } else {
                IconName::File
            })
            .small()
            .text_color(colors.muted_foreground)
            .into_any_element()
        }
    }
}

impl MainView {
    // Resolve at activation time: closing an earlier document must not retarget
    // a rendered control, and an obsolete control must not affect its neighbour.
    pub(super) fn select_navigation_document(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.documents.iter().position(|snap| snap.id == id) {
            self.switch_to(index, cx);
        }
    }

    pub(super) fn close_navigation_document(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.documents.iter().position(|snap| snap.id == id) {
            self.close_active_at(index, cx);
        }
    }

    /// Horizontal document strip. Buttons deliberately supply keyboard focus:
    /// Kit 0.7's Tab/TabBar primitives currently support pointer activation only.
    pub(super) fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme::palette(cx);
        if self.documents.is_empty() {
            return div().h_0();
        }
        let tabs = self.documents.iter().enumerate().map(|(index, snap)| {
            let id = snap.id;
            let active = Some(index) == self.active;
            let tooltip = snap.navigation_tooltip();
            // The strip is recessed and the active tab is lifted onto it, so
            // the open document reads as a sheet pulled out of the workspace
            // rather than as another button in a toolbar.
            let background = if active { colors.surface } else { colors.strip };
            let foreground = if active {
                colors.strong_foreground
            } else {
                colors.muted_foreground
            };
            let style = ButtonCustomVariant::new(cx)
                .color(background)
                .foreground(foreground)
                .hover(if active { colors.surface } else { colors.hover })
                .active(colors.chrome_active);

            div()
                .id(("doc-tab-surface", id))
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap_0()
                .h_full()
                .pr_1()
                .rounded_t(cx.theme().radius_tokens().sm)
                .bg(background)
                .when(active, |tab| {
                    // A 1px cap above the tab ties it to the surface below
                    // and stops the strip from looking like a second toolbar.
                    tab.child(
                        div()
                            .absolute()
                            .top(px(0.))
                            .left(px(6.))
                            .right(px(6.))
                            .h(px(1.5))
                            .rounded_full()
                            .bg(colors.primary),
                    )
                })
                .child(
                    Button::new(("doc-tab", id))
                        .small()
                        .custom(style)
                        .toggled(active)
                        .accessibility_label(tooltip.clone())
                        .tooltip(tooltip)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .min_w_0()
                                .h_full()
                                .child(snap.navigation_icon(colors))
                                .child(
                                    div()
                                        .max_w_48()
                                        .truncate()
                                        .font_weight(if active {
                                            gpui_kit::FontWeight::MEDIUM
                                        } else {
                                            gpui_kit::FontWeight::NORMAL
                                        })
                                        .child(snap.tab_label()),
                                ),
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.select_navigation_document(id, cx);
                        })),
                )
                .child(
                    Button::new(("doc-tab-close", id))
                        .xsmall()
                        .ghost()
                        .icon(IconName::Close)
                        .accessibility_label(format!("Close {}", snap.navigation_label()))
                        .tooltip(format!("Close {}", snap.navigation_label()))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            cx.stop_propagation();
                            view.close_navigation_document(id, cx);
                        })),
                )
        });

        div()
            .flex()
            .items_center()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .h(theme::DOCUMENT_STRIP_HEIGHT)
            .bg(colors.strip)
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .id("tab-strip")
                    .relative()
                    .flex()
                    .items_stretch()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .pl_2()
                    .pr_2()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_tooltip_retains_the_full_document_name() {
        let dir = tempfile::tempdir().unwrap();
        let name = "a-document-name-that-is-longer-than-forty-characters.csv";
        let path = dir.path().join(name);
        std::fs::write(&path, b"name\nvalue\n").unwrap();
        let doc = Arc::new(anvil_core::index_opened(
            anvil_core::source::local::open_local(&path).unwrap(),
        ));
        let snap = DocumentSnapshot::from_doc(10, doc, None, None);
        assert!(snap.tab_label().ends_with('…'));
        assert_eq!(snap.navigation_label(), name);
        assert_eq!(snap.navigation_tooltip(), name);
    }

    #[gpui_kit::test]
    fn navigation_targets_stable_document_ids(cx: &mut gpui_kit::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.csv");
        std::fs::write(&path, b"name\nvalue\n").unwrap();
        let doc = Arc::new(anvil_core::index_opened(
            anvil_core::source::local::open_local(&path).unwrap(),
        ));
        cx.update(|cx| {
            gpui_kit::init(cx);
            let view = cx.new(MainView::new);
            view.update(cx, |view, cx| {
                for id in [10, 20, 30] {
                    view.documents
                        .push(DocumentSnapshot::from_doc(id, doc.clone(), None, None));
                }
                view.active = Some(0);
                view.load_active();
                view.close_navigation_document(10, cx);
                view.select_navigation_document(30, cx);
                assert_eq!(view.active, Some(1));
                assert_eq!(view.documents[view.active.unwrap()].id, 30);
                view.close_navigation_document(10, cx);
                view.select_navigation_document(10, cx);
                assert_eq!(view.documents.len(), 2);
                assert_eq!(view.active, Some(1));
                view.close_navigation_document(20, cx);
                assert_eq!(view.active, Some(0));
                assert_eq!(view.documents[0].id, 30);
                view.close_navigation_document(30, cx);
                assert!(view.documents.is_empty());
                assert_eq!(view.active, None);
            });
        });
    }
}

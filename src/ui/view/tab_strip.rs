//! Document tab strip rendering.
//!
//! Extracted verbatim from `view.rs` (behaviour-neutral split).

use super::*;

impl MainView {
    /// Horizontal strip of tabs above the grid. Each tab shows a
    /// short label for an open document. Click a tab to switch;
    /// click the × to close. When no documents are open, the bar
    /// is empty (no row rendered).
    ///
    /// Tabs live as a normal flex child of the root, below the
    /// toolbar and above the center area. They never overlap the
    /// grid the way the search bar used to.
    pub(super) fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.documents.is_empty() {
            return div().h(px(0.));
        }
        let mut tabs: Vec<AnyElement> = Vec::with_capacity(self.documents.len());
        for (index, snap) in self.documents.iter().enumerate() {
            let is_active = Some(index) == self.active;
            let label = snap.tab_label();
            let on_select_index = index;
            let on_close_index = index;

            let active_bg = theme::BG_PANEL;
            let inactive_bg = theme::BG_TOOLBAR;
            let active_fg = theme::TEXT_STRONG;
            let inactive_fg = theme::TEXT_DIM;
            let bg = if is_active { active_bg } else { inactive_bg };
            let fg = if is_active { active_fg } else { inactive_fg };

            // Tab body - clickable to switch active. `flex_shrink_0`
            // keeps tabs at their natural width; the strip scrolls
            // horizontally rather than crushing them.
            tabs.push(
                div()
                    .id(("doc-tab", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .h(px(f32::from(theme::TAB_BAR_HEIGHT)))
                    .px_2()
                    .bg(bg)
                    .border_r_1()
                    .border_color(theme::BORDER)
                    .cursor_pointer()
                    .hover(|s| if is_active { s } else { s.bg(theme::BG_PANEL) })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.switch_to(on_select_index, cx);
                    }))
                    .child(div().text_color(fg).child(label))
                    .child(
                        div()
                            .id(("doc-tab-close", index))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::TEXT_STRONG))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |view, _, _, cx| {
                                    view.close_active_at(on_close_index, cx);
                                }),
                            )
                            .child("×"),
                    )
                    .into_any_element(),
            );
        }

        // Outer strip: a full-width bar with a horizontally scrollable
        // inner track so a long tab list stays reachable.
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(f32::from(theme::TAB_BAR_HEIGHT)))
            .w_full()
            .bg(theme::BG_TOOLBAR)
            .border_b_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .id("tab-strip")
                    .flex()
                    .flex_row()
                    .items_center()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs),
            )
    }
}

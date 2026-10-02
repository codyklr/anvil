//! Shared chrome for the right-hand auxiliary surfaces.
//!
//! Column, Diagnostics and Cell are three views of one idea: a titled
//! inspector with scannable, mostly-monospaced content. They share a header,
//! a section rhythm and a data well, so switching between them does not feel
//! like switching between three different applications.
//!
//! The hierarchy is carried by type scale and rules rather than by nested
//! cards: a muted small-caps section label, a hairline, then rows. Numbers
//! are right-aligned monospace so a column of digits lines up without the
//! reader having to parse them.

use gpui_kit::component::{button::Button, ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::{div, prelude::*, px, AnyElement, App, Div, SharedString, Stateful};

use super::theme;

/// Horizontal padding inside a panel. Kept here so all three panels agree.
pub(super) const BODY_PAD_X: f32 = theme::PANEL_INSET;

/// The panel's title row: a quiet label plus a close control.
///
/// The close control is an icon rather than a word: the panel is already
/// titled, and "Close" repeated in every panel costs more width than it
/// explains.
pub(super) fn header(
    colors: theme::Palette,
    icon: IconName,
    label: &'static str,
    close: Button,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .flex_shrink_0()
        .h(theme::PANEL_HEADER_HEIGHT)
        .pl(px(BODY_PAD_X))
        .pr(px(6.))
        .border_b_1()
        .border_color(colors.border)
        .child(Icon::new(icon).small().text_color(colors.faint_foreground))
        .child(
            div()
                .flex_1()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(colors.muted_foreground)
                .child(label.to_uppercase()),
        )
        .child(close)
}

/// The panel body container: scrolls, and owns the vertical rhythm.
pub(super) fn body(id: &'static str, content: Vec<AnyElement>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_y_scroll()
        .children(content)
}

/// The panel's identity block: the subject's name at display size, then the
/// context the numbers below it were measured in.
pub(super) fn title_block(
    cx: &App,
    colors: theme::Palette,
    subject: String,
    context: String,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px(px(BODY_PAD_X))
        .pt(px(10.))
        .pb(px(8.))
        .child(
            div()
                .min_w_0()
                .text_size(theme::PANEL_SUBJECT_SIZE)
                .line_height(theme::PANEL_SUBJECT_LINE_HEIGHT)
                .font_family(cx.theme().mono_font_family.clone())
                .text_color(colors.strong_foreground)
                .truncate()
                .child(subject),
        )
        .child(
            div()
                .min_w_0()
                .text_xs()
                .text_color(colors.muted_foreground)
                .truncate()
                .child(context),
        )
}

/// A section label with a hairline that runs to the panel's edge.
///
/// The rule is what groups the rows underneath; the label is what names the
/// group. Neither is a container, so the panel never nests.
pub(super) fn section(colors: theme::Palette, title: &str) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .mt(px(10.))
        .mb(px(4.))
        .px(px(BODY_PAD_X))
        .child(
            div()
                .flex_shrink_0()
                .text_size(theme::PANEL_SECTION_SIZE)
                .line_height(theme::PANEL_SECTION_LINE_HEIGHT)
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(colors.faint_foreground)
                .child(title.to_uppercase()),
        )
        .child(div().flex_1().h(px(1.)).bg(colors.border))
}

/// A label/value row. The value is right-aligned monospace so a column of
/// numbers scans vertically.
pub(super) fn metric(cx: &App, colors: theme::Palette, label: &str, value: &str) -> Div {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap_2()
        .px(px(BODY_PAD_X))
        .py(px(2.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(180.))
                .text_xs()
                .font_family(cx.theme().mono_font_family.clone())
                .text_color(colors.foreground)
                .text_right()
                .child(value.to_string()),
        )
}

/// A named block of verbatim data, set in a recessed well so a value is
/// never confused with a label. Used for exact min/max/longest values and
/// for sample rows.
pub(super) fn data_well(
    cx: &App,
    colors: theme::Palette,
    label: &str,
    value: &str,
    present: bool,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px(px(BODY_PAD_X))
        .pt(px(4.))
        .child(
            div()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(label.to_string()),
        )
        .child(
            div()
                .mx(px(-2.))
                .px(px(8.))
                .py(px(5.))
                .rounded(cx.theme().radius_tokens().sm)
                .bg(colors.data_well)
                .border_1()
                .border_color(colors.border)
                .child(
                    div()
                        .min_w_0()
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_color(if present {
                            colors.foreground
                        } else {
                            colors.empty_foreground
                        })
                        .child(if present {
                            value.to_string()
                        } else {
                            EMPTY_VALUE.to_string()
                        }),
                ),
        )
}

/// Shown in a data well when the column has no value to display. Byte-faithful:
/// an absent value is reported, never invented.
pub(super) const EMPTY_VALUE: &str = "no value";

/// A short caption above a group of rows.
pub(super) fn caption(colors: theme::Palette, text: &str) -> Div {
    caption_owned(colors, text.to_string())
}

/// [`caption`], for callers that already own the string.
pub(super) fn caption_owned(colors: theme::Palette, text: String) -> Div {
    div()
        .px(px(BODY_PAD_X))
        .pt(px(4.))
        .pb(px(1.))
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(text)
}

/// A quiet note where a panel has nothing to show.
///
/// It states the state and, where one exists, the action that leaves it.
/// It never advertises the application.
pub(super) fn empty_state(colors: theme::Palette, title: &str, detail: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px(px(BODY_PAD_X))
        .pt(px(16.))
        .pb(px(8.))
        .child(
            div()
                .text_sm()
                .text_color(colors.foreground)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(detail.to_string()),
        )
        .into_any_element()
}

/// Mono family accessor, so panels never hardcode a font name.
pub(super) fn mono(cx: &App) -> SharedString {
    cx.theme().mono_font_family.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_padding_is_shared() {
        // A single source of truth for panel inset; the three panels must
        // not drift apart.
        assert_eq!(BODY_PAD_X, 12.);
    }
}

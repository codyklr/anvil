//! Shared render + hit-test helpers for the editable text fields.
//!
//! GPUI 0.2.2 has no `TextInput` element, so the search and filter bars
//! render their own editable areas. This module holds the pieces both
//! share: shaping a single line for caret placement and pointer
//! hit-testing, and the `div` that draws the text, the selection
//! highlight, the caret, and a bounds-recording `canvas`.

use std::ops::Range;

use gpui::{
    canvas, div, prelude::*, px, rems, AnyElement, App, Bounds, Div, HighlightStyle, Hsla, Pixels,
    SharedString, StyledText, Window,
};

use super::theme;

/// Shape `text` with the window's current text style and return the
/// caret byte index nearest to window-space x `point_x`, using the
/// recorded bounds of the editable field. Out-of-range x clamps to the
/// nearest end.
pub(super) fn text_index_at_point(
    window: &mut Window,
    text: &str,
    bounds: Bounds<Pixels>,
    point_x: Pixels,
) -> usize {
    if text.is_empty() {
        return 0;
    }
    let shaped = shape_input_line(window, text);
    let local_x = (point_x - bounds.origin.x).max(px(0.0));
    shaped.closest_index_for_x(local_x)
}

/// Pixel x of `caret` within `text`, relative to the text start, for
/// painting the caret bar.
fn text_caret_x(window: &mut Window, text: &str, caret: usize) -> Pixels {
    if text.is_empty() {
        return px(0.0);
    }
    shape_input_line(window, text).x_for_index(caret)
}

/// Font size the editable fields render at. Must match the `text_sm()`
/// applied to the field in [`editable_text_area`]: `window.text_style()`
/// during render is the *default* style (1rem), not the inherited
/// `text_sm` (0.875rem), and shaping at the wrong size makes the caret
/// drift ahead of the glyphs as the text grows longer.
fn input_font_size(window: &Window) -> Pixels {
    rems(0.875).to_pixels(window.rem_size())
}

/// Shape a single line at the field's rendered font size. The result is
/// cached by the text system, so calling this once per frame and once
/// per mouse event is cheap.
fn shape_input_line(window: &mut Window, text: &str) -> gpui::ShapedLine {
    let style = window.text_style().clone();
    let font_size = input_font_size(window);
    let runs = [style.to_run(text.len())];
    window
        .text_system()
        .shape_line(SharedString::from(text.to_string()), font_size, &runs, None)
}

/// The inner editable area: text (with a selection highlight), a 1px
/// caret, and a hitbox-free `canvas` that reports the area's window
/// bounds through `on_bounds` each frame. The caller attaches its own
/// mouse handlers (the filter and search fields route to different
/// `MainView` methods) and wraps this in the bar's flex row.
pub(super) fn editable_text_area(
    window: &mut Window,
    text: &str,
    selection: Range<usize>,
    caret: usize,
    placeholder: &str,
    debug_id: &'static str,
    on_bounds: impl Fn(Bounds<Pixels>, &mut App) + 'static,
) -> Div {
    let caret_x = text_caret_x(window, text, caret);

    let text_content: AnyElement = if text.is_empty() {
        div()
            .text_color(theme::TEXT_DIM)
            .child(placeholder.to_string())
            .into_any_element()
    } else if selection.is_empty() {
        StyledText::new(text.to_string()).into_any_element()
    } else {
        let highlight = HighlightStyle {
            background_color: Some(Hsla::from(theme::BG_SELECTED)),
            ..Default::default()
        };
        StyledText::new(text.to_string())
            .with_highlights([(selection, highlight)])
            .into_any_element()
    };

    let bounds_recorder = canvas(
        |_, _, _| (),
        move |bounds, _, _window, app_cx| on_bounds(bounds, app_cx),
    )
    .absolute()
    .top(px(0.))
    .left(px(0.))
    .size_full();

    div()
        .relative()
        .flex_1()
        .overflow_hidden()
        // Keep this in sync with `input_font_size`, which shapes the
        // caret and hit-tests pointer clicks.
        .text_sm()
        .text_color(theme::TEXT_STRONG)
        .debug_selector(move || debug_id.to_string())
        .child(text_content)
        .child(
            div()
                .absolute()
                .left(px(f32::from(caret_x)))
                .top_0()
                .bottom_0()
                .w(px(1.5))
                .bg(theme::TEXT_STRONG),
        )
        .child(bounds_recorder)
}

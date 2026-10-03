//! Shared render + hit-test helpers for the editable text fields.
//!
//! Query bars retain their tested `TextEdit` model rather than adopting
//! Kit's retained input state. This module shares their compact field frame,
//! shaping for caret placement and pointer hit-testing, and the text,
//! selection, caret, and bounds-recording canvas.

use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    canvas, div, prelude::*, px, rems, AnyElement, App, Bounds, BoxShadow, Div, FocusHandle,
    HighlightStyle, Hsla, Pixels, SharedString, StyledText, Window,
};

use super::{text_edit::TextEdit, theme};

/// Height of a query bar. One row: label, field, live feedback, controls.
pub(super) const BAR_HEIGHT: f32 = 40.;

/// A shared label lane keeps the query editors and their feedback aligned.
pub(super) fn query_label(label: &'static str, cx: &App) -> Div {
    div()
        .w(px(46.))
        .flex_shrink_0()
        .text_xs()
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .text_color(theme::palette(cx).muted_foreground)
        .child(label)
}

/// The container for a query bar.
///
/// Find, Filter, Ask and Go to row are the same control with different
/// verbs, so they share one frame: a single 40 px row sitting above the
/// grid, a hairline under it, and a soft shadow so it reads as a surface
/// floating over the data rather than as another chrome band.
///
/// The caller adds focus, key handling and children; this owns the frame so
/// the four bars cannot drift apart.
pub(super) fn query_bar(colors: theme::Palette) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(BAR_HEIGHT))
        .px(px(8.))
        .flex_shrink_0()
        .bg(colors.panel)
        .shadow(vec![BoxShadow {
            offset: gpui_kit::point(px(0.), px(1.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
            color: colors.shadow,
            inset: false,
        }])
        .border_b_1()
        .border_color(colors.border)
}

/// A vertical hairline that separates the field's controls from the
/// window-level ones.
pub(super) fn query_rule(colors: theme::Palette) -> Div {
    div().w(px(1.)).h(px(16.)).mx(px(2.)).bg(colors.border)
}

/// Frame the existing editor without changing its text-origin hit testing.
/// Padding belongs here; the inner canvas still records only the text bounds.
pub(super) fn query_field(
    editor: impl IntoElement,
    focus: &FocusHandle,
    invalid: bool,
    window: &Window,
    cx: &App,
) -> Div {
    let colors = theme::palette(cx);
    let focused = focus.is_focused(window);
    let focus = focus.clone();
    div()
        .flex_1()
        .min_w_0()
        .h(px(24.))
        .flex()
        .items_center()
        .px_2()
        .bg(colors.surface)
        .border_1()
        .border_color(if focused {
            cx.theme().ring
        } else if invalid {
            colors.danger
        } else {
            colors.border
        })
        .rounded(cx.theme().radius_tokens().sm)
        .cursor_text()
        .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
            window.focus(&focus, cx);
        })
        .child(editor)
}

pub(super) fn query_feedback(text: String, color: Hsla) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_xs()
        .text_color(color)
        .child(text)
}

/// Windows reports Space as a named key without printable `key_char`.
/// Prefer supplied text so other backends still insert it exactly once.
pub(super) fn printable_key_text(keystroke: &gpui_kit::Keystroke) -> Option<&str> {
    let text = keystroke
        .key_char
        .as_deref()
        .or_else(|| (keystroke.key == "space").then_some(" "))?;
    (!text.is_empty() && text.chars().all(|c| !c.is_control())).then_some(text)
}

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
fn shape_input_line(window: &mut Window, text: &str) -> gpui_kit::ShapedLine {
    let style = window.text_style().clone();
    let font_size = input_font_size(window);
    let runs = [style.to_run(text.len())];
    window
        .text_system()
        .shape_line(SharedString::from(text.to_string()), font_size, &runs, None)
}

/// Frame-local presentation of an existing editor; no retained input state.
pub(super) struct InputPresentation<'a> {
    pub(super) edit: &'a TextEdit,
    pub(super) focus: &'a FocusHandle,
    pub(super) placeholder: &'a str,
    pub(super) debug_id: &'static str,
}

/// The inner editable area: text (with a selection highlight), a focused
/// caret, and a hitbox-free `canvas` that reports the area's window
/// bounds through `on_bounds` each frame. The caller attaches its own
/// mouse handlers (the filter and search fields route to different
/// `MainView` methods) and wraps this in the bar's flex row.
pub(super) fn editable_text_area(
    window: &mut Window,
    cx: &App,
    presentation: InputPresentation<'_>,
    on_bounds: impl Fn(Bounds<Pixels>, &mut App) + 'static,
) -> Div {
    let InputPresentation {
        edit,
        focus,
        placeholder,
        debug_id,
    } = presentation;
    let colors = theme::palette(cx);
    let text = edit.text();
    let selection = edit.selection();
    let caret_x = focus
        .is_focused(window)
        .then(|| text_caret_x(window, text, edit.caret()));

    let text_content: AnyElement = if text.is_empty() {
        div()
            .text_color(colors.muted_foreground)
            .child(placeholder.to_string())
            .into_any_element()
    } else if selection.is_empty() {
        StyledText::new(text.to_string()).into_any_element()
    } else {
        let highlight = HighlightStyle {
            background_color: Some(colors.selection),
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
        .min_w_0()
        .overflow_hidden()
        // Keep this in sync with `input_font_size`, which shapes the
        // caret and hit-tests pointer clicks.
        .text_sm()
        // Kit's text_sm sets only font size. Do not inherit shell line
        // height or wrap: shaping and hit testing both describe one line.
        .h(px(18.))
        .line_height(px(18.))
        .whitespace_nowrap()
        .text_color(colors.strong_foreground)
        .debug_selector(move || debug_id.to_string())
        .child(text_content)
        .when_some(caret_x, |area, caret_x| {
            area.child(
                div()
                    .absolute()
                    .left(caret_x)
                    .top_0()
                    .bottom_0()
                    .w(px(1.5))
                    .bg(colors.strong_foreground)
                    .debug_selector(move || format!("{debug_id}-caret")),
            )
        })
        .child(bounds_recorder)
}

#[cfg(test)]
mod field_tests {
    use super::*;

    use gpui_kit::{Context, FocusHandle, MouseButton, MouseDownEvent, Render, TestAppContext};

    struct EditorHarness {
        focus: FocusHandle,
        edit: TextEdit,
        bounds: Option<Bounds<Pixels>>,
    }

    impl Render for EditorHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let entity = cx.entity();
            let editor = editable_text_area(
                window,
                cx,
                InputPresentation {
                    edit: &self.edit,
                    focus: &self.focus,
                    placeholder: "Query",
                    debug_id: "query-inner",
                },
                move |bounds, cx| {
                    entity.update(cx, |view, _| view.bounds = Some(bounds));
                },
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, _| {
                    let bounds = this.bounds.expect("text bounds");
                    let ix =
                        text_index_at_point(window, this.edit.text(), bounds, event.position.x);
                    this.edit.set_caret(ix, false);
                }),
            );
            div()
                .p_3()
                .w(px(220.))
                .line_height(px(48.))
                .text_sm()
                .track_focus(&self.focus)
                .child(
                    query_field(editor, &self.focus, false, window, cx)
                        .debug_selector(|| "query-frame".to_string()),
                )
        }
    }

    #[gpui_kit::test]
    async fn query_geometry_contains_text_and_caret_without_wrapping(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            gpui_kit::init(cx);
            EditorHarness {
                focus: cx.focus_handle().tab_stop(true),
                edit: TextEdit::default(),
                bounds: None,
            }
        });
        for (text, selected) in [
            (String::new(), false),
            ("héllo".to_string(), false),
            ("a long query with spaces ".repeat(20), false),
            ("a long selected query with spaces ".repeat(20), true),
        ] {
            cx.update_window_entity(&view, |view, window, cx| {
                view.edit = TextEdit::from(text.as_str());
                if selected {
                    view.edit.select_all();
                }
                // Keep the caret visible while testing horizontal overflow.
                view.edit.set_caret(0, selected);
                window.focus(&view.focus, cx);
                cx.notify();
            });
            cx.run_until_parked();
            let frame = cx.debug_bounds("query-frame").expect("field frame");
            let inner = cx.debug_bounds("query-inner").expect("text area");
            let caret = cx.debug_bounds("query-inner-caret").expect("caret");
            assert_eq!(frame.size.height, px(24.));
            assert_eq!(inner.size.height, px(18.));
            assert_eq!(inner.center().y, frame.center().y);
            assert!(inner.origin.x > frame.origin.x);
            assert!(inner.right() < frame.right());
            assert!(inner.origin.y > frame.origin.y);
            assert!(inner.bottom() < frame.bottom());
            assert_eq!(caret.origin.y, inner.origin.y);
            assert_eq!(caret.size.height, inner.size.height);
            cx.update_window_entity(&view, |view, _, _| {
                assert_eq!(view.bounds, Some(inner));
                assert_eq!(view.edit.text(), text.as_str());
            });
        }
    }

    #[gpui_kit::test]
    async fn query_caret_follows_focus_without_changing_the_editor(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            gpui_kit::init(cx);
            let mut edit = TextEdit::from("héllo");
            edit.select_all();
            EditorHarness {
                focus: cx.focus_handle().tab_stop(true),
                edit,
                bounds: None,
            }
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("query-inner-caret").is_none());

        cx.update_window_entity(&view, |view, window, cx| window.focus(&view.focus, cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("query-inner-caret").is_some());

        cx.update_window_entity(&view, |_, window, cx| window.blur(cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("query-inner-caret").is_none());
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.edit.text(), "héllo");
            assert_eq!(view.edit.selection(), 0.."héllo".len());
            assert_eq!(view.edit.caret(), "héllo".len());
        });
    }

    #[gpui_kit::test]
    async fn query_frame_preserves_text_origin_and_restores_editor_focus(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            gpui_kit::init(cx);
            EditorHarness {
                focus: cx.focus_handle().tab_stop(true),
                edit: "héllo".into(),
                bounds: None,
            }
        });
        cx.run_until_parked();
        let position = cx.update_window_entity(&view, |view, window, _| {
            let bounds = view.bounds.expect("text bounds");
            // A UTF-8 byte boundary measured from the inner text origin,
            // not from the padded frame, must round-trip through hit testing.
            gpui_kit::point(
                bounds.origin.x + text_caret_x(window, view.edit.text(), 3),
                bounds.center().y,
            )
        });
        cx.simulate_mouse_down(position, MouseButton::Left, gpui_kit::Modifiers::none());
        cx.simulate_mouse_up(position, MouseButton::Left, gpui_kit::Modifiers::none());
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, window, _| {
            assert_eq!(view.edit.caret(), 3);
            assert!(view.focus.is_focused(window));
        });
    }
}

#[cfg(test)]
mod keyboard_tests {
    use super::printable_key_text;
    use gpui_kit::{Keystroke, Modifiers};

    #[test]
    fn windows_space_without_key_char_is_printable() {
        let key = Keystroke {
            modifiers: Modifiers::default(),
            key: "space".into(),
            key_char: None,
        };
        assert_eq!(printable_key_text(&key), Some(" "));
    }

    #[test]
    fn supplied_text_is_preserved_and_control_keys_are_ignored() {
        let mut key = Keystroke {
            modifiers: Modifiers::default(),
            key: "space".into(),
            key_char: Some(" ".into()),
        };
        assert_eq!(printable_key_text(&key), Some(" "));
        key.key_char = Some("Ã©".into());
        assert_eq!(printable_key_text(&key), Some("Ã©"));
        for text in ["", "\t", "\n"] {
            key.key_char = Some(text.into());
            assert_eq!(printable_key_text(&key), None);
        }
        key.key = "enter".into();
        key.key_char = None;
        assert_eq!(printable_key_text(&key), None);
    }
}

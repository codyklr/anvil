//! Go-to-row input bar (Cmd/Ctrl+G).
//!
//! A small one-line bar that reuses the shared `TextEdit` buffer. Enter
//! resolves the typed number as a 1-based *display* row (the order the
//! grid currently shows, after any filter or sort), clamps it to the
//! visible range, moves the cursor to that display position and scrolls
//! it to the top. Esc closes the bar without changing the selection.

use gpui_kit::component::{
    button::{Button, ButtonVariants},
    Disableable, IconName, Sizable,
};

use super::*;

impl MainView {
    /// Open the go-to-row bar (Cmd/Ctrl+G). No-op with no document.
    pub(super) fn open_goto(&mut self, cx: &mut Context<Self>) {
        if self.model.is_none() {
            return;
        }
        self.goto_open = true;
        self.goto_error = None;
        self.focus_goto_on_next_frame = true;
        cx.notify();
    }

    /// Close the bar and return focus to the grid. Does not change the
    /// selection.
    pub(super) fn close_goto(&mut self, cx: &mut Context<Self>) {
        self.goto_open = false;
        self.goto_error = None;
        self.focus_grid_on_next_frame = true;
        cx.notify();
    }

    /// Number of rows currently displayed: the filtered/sorted view when
    /// one is active, otherwise every record.
    fn displayed_row_count(&self) -> u64 {
        self.view_row_count()
    }

    /// Resolve a 1-based display row number to a 0-based **display
    /// position**. Returns `None` for an empty or non-numeric entry. The
    /// number is clamped to `[1, displayed_row_count]` so an out-of-range
    /// jump lands on the nearest row instead of doing nothing.
    fn display_row_for_input(&self, input: &str) -> Option<u64> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }
        let n: u64 = trimmed.parse().ok()?;
        let total = self.displayed_row_count();
        if total == 0 {
            return None;
        }
        Some(n.clamp(1, total) - 1)
    }

    /// Commit the input: jump to the requested display row, keeping the
    /// current column, and scroll it to the top. Invalid input shows an
    /// inline error and leaves the selection untouched.
    pub(super) fn commit_goto(&mut self, cx: &mut Context<Self>) {
        let input = self.goto_input.text().to_string();
        let Some(display) = self.display_row_for_input(&input) else {
            self.goto_error = Some("enter a row number".to_string());
            cx.notify();
            return;
        };
        let Some(model) = &self.model else {
            return;
        };
        let cols = model.doc.column_count();
        if cols == 0 {
            return;
        }
        let col = self
            .selection
            .cursor()
            .map(|c| c.column_index.min(cols - 1))
            .unwrap_or(0);
        // The selection is in display space, so the resolved position is
        // both the cursor row and the list offset.
        self.selection.set(CellPos::new(display, col));
        self.v_scroll
            .scroll_to_item_strict(display as usize, ScrollStrategy::Top);
        self.ensure_column_visible(col, cx);
        self.kick_inspector(cx);
        self.goto_error = None;
        self.close_goto(cx);
    }

    /// The go-to-row bar's key handler. Enter commits, Esc closes, and
    /// the usual single-line editing keys modify the input.
    pub(super) fn on_goto_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.goto_open {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;

        if key == "escape" {
            self.close_goto(cx);
            return;
        }

        if key == "enter" {
            self.commit_goto(cx);
            return;
        }

        // Cmd/Ctrl+A selects the whole input, matching the other bars.
        if key == "a" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
            self.goto_input.select_all();
            cx.notify();
            return;
        }

        let extend = modifiers.shift;
        let word_move = if cfg!(target_os = "macos") {
            modifiers.alt
        } else {
            modifiers.control
        };
        let line_jump = cfg!(target_os = "macos") && modifiers.secondary();

        match key {
            "left" => {
                if line_jump {
                    self.goto_input.move_home(extend);
                } else if word_move {
                    self.goto_input.move_word_left(extend);
                } else {
                    self.goto_input.move_left(extend);
                }
                cx.notify();
                return;
            }
            "right" => {
                if line_jump {
                    self.goto_input.move_end(extend);
                } else if word_move {
                    self.goto_input.move_word_right(extend);
                } else {
                    self.goto_input.move_right(extend);
                }
                cx.notify();
                return;
            }
            "home" => {
                self.goto_input.move_home(extend);
                cx.notify();
                return;
            }
            "end" => {
                self.goto_input.move_end(extend);
                cx.notify();
                return;
            }
            "backspace" if !modifiers.control && !modifiers.alt => {
                self.goto_input.backspace();
                self.goto_error = None;
                cx.notify();
                return;
            }
            "delete" if !modifiers.control && !modifiers.alt => {
                self.goto_input.delete();
                self.goto_error = None;
                cx.notify();
                return;
            }
            _ => {}
        }

        if !modifiers.control && !modifiers.alt && !modifiers.function && !modifiers.secondary() {
            if let Some(ch) = &event.keystroke.key_char {
                if !ch.is_empty() && ch.chars().all(|c| !c.is_control()) {
                    self.goto_input.insert(ch);
                    self.goto_error = None;
                    cx.notify();
                }
            }
        }
    }

    /// Mouse press in the go-to-row input: place the caret at the
    /// clicked byte index and arm drag-select.
    pub(super) fn goto_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.goto_text_bounds else {
            return;
        };
        let index =
            input::text_index_at_point(window, self.goto_input.text(), bounds, event.position.x);
        self.goto_input.set_caret(index, event.modifiers.shift);
        self.goto_dragging = true;
        cx.notify();
    }

    /// Mouse drag in the go-to-row input: extend the selection.
    pub(super) fn goto_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.goto_dragging || !event.dragging() {
            return;
        }
        let Some(bounds) = self.goto_text_bounds else {
            return;
        };
        let index =
            input::text_index_at_point(window, self.goto_input.text(), bounds, event.position.x);
        self.goto_input.set_caret(index, true);
        cx.notify();
    }

    pub(super) fn goto_mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.goto_dragging {
            self.goto_dragging = false;
            cx.notify();
        }
    }

    /// The go-to-row bar. Like the search and filter bars it is a
    /// normal-flow child above the grid and carries its own key context
    /// and focus handle.
    pub(super) fn render_goto_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let colors = theme::palette(cx);
        if !self.goto_open {
            return None;
        }
        let (status_text, status_color) = match &self.goto_error {
            Some(error) => (error.clone(), colors.danger),
            None => (
                format!("{} rows", format_count(self.displayed_row_count())),
                colors.muted_foreground,
            ),
        };

        Some(
            input::query_bar(colors)
                .min_w_0()
                .key_context("GoToRow")
                .track_focus(&self.goto_focus_handle.clone().tab_stop(true))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if this.goto_focus_handle.is_focused(window) || event.keystroke.key == "escape"
                    {
                        this.on_goto_key_down(event, window, cx);
                    }
                }))
                .child(input::query_label("Row", cx))
                .child(input::query_field(
                    input::editable_text_area(
                        window,
                        cx,
                        input::InputPresentation {
                            edit: &self.goto_input,
                            focus: &self.goto_focus_handle,
                            placeholder: "row number",
                            debug_id: "anvil-goto-input",
                        },
                        {
                            let entity = cx.entity();
                            move |bounds, app_cx| {
                                entity.update(app_cx, |view, _| {
                                    view.goto_text_bounds = Some(bounds);
                                });
                            }
                        },
                    )
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(Self::goto_mouse_down),
                    )
                    .on_mouse_move(cx.listener(Self::goto_mouse_move))
                    .on_mouse_up(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|this, _ev: &MouseUpEvent, _window, cx| this.goto_mouse_up(cx)),
                    ),
                    &self.goto_focus_handle,
                    self.goto_error.is_some(),
                    window,
                    cx,
                ))
                .child(input::query_feedback(status_text, status_color))
                .child(input::query_rule(colors))
                .child(
                    Button::new("goto-commit")
                        .small()
                        .primary()
                        .icon(IconName::ArrowRight)
                        .label("Go")
                        .disabled(self.goto_input.is_empty() || self.displayed_row_count() == 0)
                        .on_click(cx.listener(|this, _, _, cx| this.commit_goto(cx))),
                )
                .child(
                    Button::new("goto-close")
                        .small()
                        .ghost()
                        .debug_selector(|| "goto-close".to_string())
                        .icon(IconName::Close)
                        .accessibility_label("Close Go to row")
                        .tooltip("Close Go to row · Esc")
                        .on_click(cx.listener(|this, _, _, cx| this.close_goto(cx))),
                )
                .into_any_element(),
        )
    }
}

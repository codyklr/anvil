//! Natural-language Ask bar (Cmd/Ctrl+Shift+L).
//!
//! Type a phrase, see the Tier 1 parse previewed as filter syntax, and
//! accept it into the normal filter bar. It never applies a query
//! silently: the preview is shown as you type, and Enter only accepts a
//! phrase that produced an expression. See
//! `docs/adr/0001-natural-language-ask.md`.

use std::cell::RefCell;

use gpui_kit::component::{
    button::{Button, ButtonVariants},
    Disableable, IconName, Sizable,
};

use super::*;

impl MainView {
    pub(super) fn invalidate_ask(&mut self) {
        self.ask_generation = self.ask_generation.wrapping_add(1);
        self.ask_task = None;
        self.ask_loading = false;
        self.ask_model_error = None;
        self.ask_schema = None;
        self.ask_result = None;
        self.ask_open = false;
        self.ask_input.clear();
        self.focus_ask_on_next_frame = false;
    }

    pub(super) fn open_ask(&mut self, cx: &mut Context<Self>) {
        self.ask_open = true;
        self.build_ask_schema();
        self.refresh_ask();
        self.focus_ask_on_next_frame = true;
        cx.notify();
    }

    pub(super) fn close_ask(&mut self, cx: &mut Context<Self>) {
        self.ask_open = false;
        self.ask_result = None;
        self.ask_loading = false;
        self.ask_model_error = None;
        self.ask_generation = self.ask_generation.wrapping_add(1);
        self.focus_grid_on_next_frame = true;
        self.ask_task = None;
        cx.notify();
    }

    /// Build the parser's schema context for the active document. Column
    /// types come from `schema::infer`; value samples are fetched lazily
    /// during parsing.
    fn build_ask_schema(&mut self) {
        let Some(model) = self.model.clone() else {
            self.ask_schema = None;
            return;
        };
        let profiles = infer_schema(&model.doc);
        let headers = &model.doc.meta.headers;
        let columns = (0..model.doc.column_count())
            .map(|i| NlColumn {
                name: anvil_core::column_label(headers, i),
                column_type: profiles
                    .get(i)
                    .map(|p| p.column_type)
                    .unwrap_or(anvil_core::ColumnType::Text),
            })
            .collect();
        self.ask_schema = Some(NlSchema {
            columns,
            aliases: Vec::new(),
        });
    }

    /// Re-parse the current input. Tier 1 is cheap, so this runs on every
    /// edit and keeps the preview in step with the typing.
    pub(super) fn refresh_ask(&mut self) {
        self.ask_generation = self.ask_generation.wrapping_add(1);
        self.ask_task = None;
        self.ask_loading = false;
        self.ask_model_error = None;
        let (Some(schema), Some(model)) = (self.ask_schema.clone(), self.model.clone()) else {
            self.ask_result = None;
            return;
        };
        let phrase = self.ask_input.text().to_string();
        // `parse` takes `Fn`, so interior mutability lets the closure
        // reuse the filter value cache across one parse.
        let cache = RefCell::new(std::mem::take(&mut self.filter_value_cache));
        let values = |column: usize| -> Vec<String> {
            let mut cache = cache.borrow_mut();
            cache
                .entry(column)
                .or_insert_with(|| sample_distinct(&model.doc, column, 50_000, 1000))
                .clone()
        };
        let result = parse_nl(&phrase, &schema, &values);
        self.filter_value_cache = cache.into_inner();
        self.ask_result = Some(result);
    }

    /// Accept the parsed expression: write it into the filter bar and run
    /// the normal filter path. Incomplete interpretations stay in Ask.
    pub(super) fn accept_ask(&mut self, cx: &mut Context<Self>) {
        let Some(expr) = self
            .ask_result
            .as_ref()
            .filter(|r| r.clean && !self.ask_loading)
            .and_then(|r| r.expr.clone())
        else {
            return;
        };
        let headers = self
            .model
            .as_ref()
            .map(|m| m.doc.meta.headers.clone())
            .unwrap_or_default();
        let text = describe_nl(&expr, &headers);
        if anvil_core::parse_filter_expr(
            &text,
            &headers,
            self.model.as_ref().map_or(0, |m| m.doc.column_count()),
        )
        .ok()
        .as_ref()
            != Some(&expr)
        {
            self.ask_model_error = Some(
                "This preview cannot be represented unambiguously. Edit the filter directly."
                    .into(),
            );
            cx.notify();
            return;
        }
        self.close_ask(cx);
        self.focus_grid_on_next_frame = false;
        self.switch_to_filter(cx);
        self.filter_input = text.into();
        self.apply_filter_input(cx);
    }

    /// Escalate to the optional Tier 2 interpreter (Ctrl/Cmd+Enter). A
    /// no-op when the deterministic parse was already clean or no
    /// interpreter is available. Runs off the UI thread.
    pub(super) fn kick_ask_embedded(&mut self, cx: &mut Context<Self>) {
        if self.ask_loading {
            return;
        }
        if self.ask_result.as_ref().is_some_and(|result| result.clean) {
            return;
        }
        let (Some(schema), Some(interpreter), Some(model)) = (
            self.ask_schema.clone(),
            self.ask_interpreter.clone(),
            self.model.clone(),
        ) else {
            return;
        };
        let phrase = self.ask_input.text().to_string();
        self.ask_generation = self.ask_generation.wrapping_add(1);
        let generation = self.ask_generation;
        self.ask_loading = true;
        self.ask_model_error = None;
        cx.notify();

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let outcome = executor
                .spawn(async move {
                    let cache =
                        RefCell::new(std::collections::HashMap::<usize, Vec<String>>::new());
                    let values = |column: usize| -> Vec<String> {
                        let mut cache = cache.borrow_mut();
                        cache
                            .entry(column)
                            .or_insert_with(|| sample_distinct(&model.doc, column, 50_000, 1000))
                            .clone()
                    };
                    interpreter(&phrase, &schema, &values)
                })
                .await;
            this.update(cx, |view, cx| {
                if view.ask_generation != generation {
                    return;
                }
                view.ask_loading = false;
                match outcome {
                    Ok(expr) => {
                        view.ask_result = Some(NlResult {
                            expr: Some(expr),
                            unmatched: Vec::new(),
                            clean: true,
                            from_model: true,
                        });
                    }
                    Err(message) => view.ask_model_error = Some(message),
                }
                cx.notify();
            })
            .ok();
        });
        self.ask_task = Some(task);
    }

    pub(super) fn on_ask_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ask_open {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let extend = modifiers.shift;

        if key == "escape" {
            self.close_ask(cx);
            return;
        }
        if key == "enter" {
            if (modifiers.secondary() || modifiers.control) && !modifiers.alt {
                self.kick_ask_embedded(cx);
            } else {
                self.accept_ask(cx);
            }
            return;
        }
        // The other bars' shortcuts switch to them.
        if key == "f" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
            self.switch_to_search(cx);
            return;
        }
        if key == "l"
            && modifiers.secondary()
            && !modifiers.alt
            && !modifiers.function
            && !modifiers.shift
        {
            self.switch_to_filter(cx);
            return;
        }
        if key == "a" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
            self.ask_input.select_all();
            cx.notify();
            return;
        }

        let word_move = if cfg!(target_os = "macos") {
            modifiers.alt
        } else {
            modifiers.control
        };
        let line_jump = cfg!(target_os = "macos") && modifiers.secondary();

        match key {
            "left" => {
                if line_jump {
                    self.ask_input.move_home(extend);
                } else if word_move {
                    self.ask_input.move_word_left(extend);
                } else {
                    self.ask_input.move_left(extend);
                }
                cx.notify();
                return;
            }
            "right" => {
                if line_jump {
                    self.ask_input.move_end(extend);
                } else if word_move {
                    self.ask_input.move_word_right(extend);
                } else {
                    self.ask_input.move_right(extend);
                }
                cx.notify();
                return;
            }
            "home" => {
                self.ask_input.move_home(extend);
                cx.notify();
                return;
            }
            "end" => {
                self.ask_input.move_end(extend);
                cx.notify();
                return;
            }
            "backspace" if !modifiers.control && !modifiers.alt => {
                self.ask_input.backspace();
                self.refresh_ask();
                cx.notify();
                return;
            }
            "delete" if !modifiers.control && !modifiers.alt => {
                self.ask_input.delete();
                self.refresh_ask();
                cx.notify();
                return;
            }
            _ => {}
        }

        if !modifiers.control && !modifiers.alt && !modifiers.function && !modifiers.secondary() {
            if let Some(ch) = super::super::input::printable_key_text(&event.keystroke) {
                self.ask_input.insert(ch);
                self.refresh_ask();
                cx.notify();
            }
        }
    }

    pub(super) fn ask_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.ask_text_bounds else {
            return;
        };
        let index =
            input::text_index_at_point(window, self.ask_input.text(), bounds, event.position.x);
        self.ask_input.set_caret(index, event.modifiers.shift);
        self.ask_dragging = true;
        cx.notify();
    }

    pub(super) fn ask_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ask_dragging || !event.dragging() {
            return;
        }
        let Some(bounds) = self.ask_text_bounds else {
            return;
        };
        let index =
            input::text_index_at_point(window, self.ask_input.text(), bounds, event.position.x);
        self.ask_input.set_caret(index, true);
        cx.notify();
    }

    pub(super) fn ask_mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.ask_dragging {
            self.ask_dragging = false;
            cx.notify();
        }
    }

    /// Ask bar (Cmd/Ctrl+Shift+L). A normal-flow child above the grid,
    /// reusing the shared editable-text area. Shows a live preview of the
    /// parsed expression, or what could not be parsed.
    pub(super) fn render_ask_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let colors = theme::palette(cx);
        if !self.ask_open {
            return None;
        }
        let (status, status_color) = self.ask_status_text(cx);

        let text_area = input::editable_text_area(
            window,
            cx,
            input::InputPresentation {
                edit: &self.ask_input,
                focus: &self.ask_focus_handle,
                placeholder: "type a question, e.g. country is US and amount over 100",
                debug_id: "anvil-ask-input",
            },
            {
                let entity = cx.entity();
                move |bounds, app_cx| {
                    entity.update(app_cx, |view, _| {
                        view.ask_text_bounds = Some(bounds);
                    });
                }
            },
        )
        .on_mouse_down(
            gpui_kit::MouseButton::Left,
            cx.listener(Self::ask_mouse_down),
        )
        .on_mouse_move(cx.listener(Self::ask_mouse_move))
        .on_mouse_up(
            gpui_kit::MouseButton::Left,
            cx.listener(|this, _ev: &MouseUpEvent, _window, cx| this.ask_mouse_up(cx)),
        );

        Some(
            input::query_bar(colors)
                .min_w_0()
                .key_context("Ask")
                .track_focus(&self.ask_focus_handle.clone().tab_stop(true))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if this.ask_focus_handle.is_focused(window) || event.keystroke.key == "escape" {
                        this.on_ask_key_down(event, window, cx);
                    }
                }))
                .child(input::query_label("Ask", cx))
                .child(input::query_field(
                    text_area,
                    &self.ask_focus_handle,
                    self.ask_model_error.is_some(),
                    window,
                    cx,
                ))
                .child(input::query_feedback(
                    if self
                        .ask_result
                        .as_ref()
                        .is_some_and(|result| result.clean && result.expr.is_some())
                        && !self.ask_loading
                        && self.ask_model_error.is_none()
                    {
                        format!("Preview: {status}")
                    } else {
                        status
                    },
                    status_color,
                ))
                .child(input::query_rule(colors))
                .when(self.ask_interpreter.is_some(), |row| {
                    row.child(
                        Button::new("ask-interpret")
                            .small()
                            .ghost()
                            .icon(IconName::Bot)
                            .label("Interpret")
                            .tooltip("Use the local model · Ctrl/Cmd+Enter")
                            .loading(self.ask_loading)
                            .disabled(
                                self.ask_input.is_empty()
                                    || self.ask_result.as_ref().is_some_and(|result| result.clean),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.kick_ask_embedded(cx))),
                    )
                })
                .child(
                    Button::new("ask-accept")
                        .small()
                        .primary()
                        .icon(IconName::Check)
                        .label("Apply filter")
                        .tooltip("Apply the previewed filter · Enter")
                        .disabled(
                            self.ask_loading
                                || !self
                                    .ask_result
                                    .as_ref()
                                    .is_some_and(|result| result.clean && result.expr.is_some()),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.accept_ask(cx))),
                )
                .child(
                    Button::new("ask-close")
                        .small()
                        .ghost()
                        .debug_selector(|| "ask-close".to_string())
                        .icon(IconName::Close)
                        .accessibility_label("Close Ask")
                        .tooltip("Close Ask · Esc")
                        .on_click(cx.listener(|this, _, _, cx| this.close_ask(cx))),
                )
                .into_any_element(),
        )
    }

    /// Preview/status for the Ask bar: the parsed expression, or the
    /// spans that could not be understood.
    pub(super) fn ask_status_text(&self, cx: &gpui_kit::App) -> (String, gpui_kit::Hsla) {
        let colors = theme::palette(cx);
        const HINT: &str = "Describe the rows you want, e.g. country is US";
        if self.ask_loading {
            return (
                "Interpreting with the local model".to_string(),
                colors.muted_foreground,
            );
        }
        if let Some(error) = &self.ask_model_error {
            return (format!("Model: {error}"), colors.danger);
        }
        match &self.ask_result {
            Some(result) => {
                if let Some(expr) = &result.expr {
                    let headers = self
                        .model
                        .as_ref()
                        .map(|m| m.doc.meta.headers.as_slice())
                        .unwrap_or(&[]);
                    let mut text = describe_nl(expr, headers);
                    if result.clean {
                        if result.from_model {
                            text = format!("{text}  (local model)");
                        }
                        (text, colors.foreground)
                    } else {
                        let note = if result.unmatched.is_empty() {
                            "check the phrase".to_string()
                        } else {
                            format!("unrecognised: {}", result.unmatched.join(", "))
                        };
                        (format!("{text}  ({note})"), colors.warning)
                    }
                } else if !result.unmatched.is_empty() {
                    (
                        format!("couldn't parse: {}", result.unmatched.join(", ")),
                        colors.warning,
                    )
                } else {
                    (HINT.to_string(), colors.muted_foreground)
                }
            }
            None => (HINT.to_string(), colors.muted_foreground),
        }
    }
}

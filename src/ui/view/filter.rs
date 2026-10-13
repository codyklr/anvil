//! Filter bar: predicate application, rendering, and mouse/keyboard
//! editing.
//!
//! The filter predicate and its per-tab state stay on `MainView`; this
//! module holds the filter half of its inherent methods. Data-side
//! parsing/evaluation lives in `anvil_core::query::filter`.

use gpui_kit::component::{
    button::{Button, ButtonVariants},
    Disableable, IconName, Sizable,
};

use super::*;

impl MainView {
    /// Open the filter bar (Cmd/Ctrl+L). The input already holds the
    /// tab's current predicate so the user can edit rather than
    /// retype. Nothing is applied until Enter.
    pub(super) fn open_filter(&mut self, cx: &mut Context<Self>) {
        self.filter_open = true;
        self.filter_error = None;
        self.filter_popup_open = false;
        self.refresh_filter_suggestions();
        self.focus_filter_on_next_frame = true;
        cx.notify();
    }

    /// Close the filter bar without changing the applied predicate.
    pub(super) fn close_filter(&mut self, cx: &mut Context<Self>) {
        self.filter_open = false;
        self.filter_error = None;
        self.filter_popup_open = false;
        self.focus_grid_on_next_frame = true;
        cx.notify();
    }

    /// Parse the filter input and, when valid, evaluate it on the
    /// background executor. An empty input clears the filter. A parse
    /// error is shown inline and leaves the previous predicate in
    /// place.
    pub(super) fn apply_filter_input(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.model.as_ref().map(|m| m.doc.clone()) else {
            return;
        };
        let input = self.filter_input.text().trim().to_string();
        if input.is_empty() {
            self.clear_filter(cx);
            return;
        }
        let expr = match parse_filter_expr(&input, &doc.meta.headers, doc.column_count()) {
            Ok(expr) => expr,
            Err(message) => {
                self.filter_error = Some(message);
                cx.notify();
                return;
            }
        };
        self.filter_applied = Some(expr.clone());
        self.filter_error = None;
        self.filter_generation = self.filter_generation.wrapping_add(1);
        let generation = self.filter_generation;
        // Clear the previous result so the grid does not show stale
        // rows while the new predicate runs.
        self.filter_rows = None;

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let rows = executor
                .spawn(async move { apply_filter_expr(&doc, &expr) })
                .await;
            this.update(cx, |view, cx| {
                if view.filter_generation != generation {
                    return;
                }
                view.filter_rows = Some(Arc::new(rows));
                // The visible row set changed under the selection.
                view.clamp_selection_to_view();
                if view.sort_spec.is_some() {
                    view.kick_sort(cx);
                }
                // A scoped inspector must reflect the new visible set.
                view.kick_inspector(cx);
                cx.notify();
            })
            .ok();
        });
        self.filter_task = Some(task);
        cx.notify();
    }

    /// Remove the applied filter and restore the full document.
    pub(super) fn clear_filter(&mut self, cx: &mut Context<Self>) {
        self.filter_input.clear();
        self.filter_applied = None;
        self.filter_rows = None;
        self.filter_error = None;
        self.filter_generation = self.filter_generation.wrapping_add(1);
        self.filter_task = None;
        self.filter_suggestions.clear();
        self.filter_popup_open = false;
        // Re-sort the full set if a sort is active.
        self.kick_sort(cx);
        // The inspector now describes the whole document again.
        self.kick_inspector(cx);
        cx.notify();
    }

    /// Recompute the autocomplete rows for the current input and caret.
    /// Pure and cheap; the only scan is the one-time, bounded, per-column
    /// value sample cached in `filter_value_cache`.
    pub(super) fn refresh_filter_suggestions(&mut self) {
        let Some(model) = self.model.clone() else {
            self.filter_suggestions.clear();
            self.filter_popup_open = false;
            return;
        };
        let input = self.filter_input.text().to_string();
        let caret = self.filter_input.caret();
        let headers = &model.doc.meta.headers;
        let column_count = model.doc.column_count();
        let ctx = suggest_context(&input, caret, headers, column_count);

        let values = match &ctx.slot {
            Slot::Value {
                column: Some(col), ..
            } => {
                if !self.filter_value_cache.contains_key(col) {
                    // Sample evenly across the whole file (not just the
                    // head) so a value that only appears late, e.g. in a
                    // sorted column, is still suggestible.
                    let sampled = sample_distinct(&model.doc, *col, 50_000, 1000);
                    self.filter_value_cache.insert(*col, sampled);
                }
                self.filter_value_cache.get(col).map(|v| v.as_slice())
            }
            _ => None,
        };
        let items = suggest_candidates(&ctx, headers, column_count, values);
        let is_column = matches!(ctx.slot, Slot::Column);
        self.filter_suggestions = items
            .into_iter()
            .map(|text| {
                // A column name with whitespace (or a quote) must be quoted
                // to stay one token, otherwise the accepted text splits into
                // several words and the column cannot be resolved.
                let quote =
                    is_column && (text.chars().any(char::is_whitespace) || text.contains('"'));
                FilterSuggestion {
                    text,
                    replace: ctx.replace.clone(),
                    quote,
                }
            })
            .collect();
        self.filter_suggestion_selected = 0;
        self.filter_suggestion_scroll.scroll_to_item(0);
    }

    /// Accept suggestion `index`: replace the partial token and add a
    /// separating space when the following character is not already a
    /// separator. Closes the popup so the next Enter applies the filter.
    pub(super) fn accept_filter_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.filter_suggestions.get(index).cloned() else {
            return;
        };
        let insert = if item.quote {
            format!("\"{}\"", item.text.replace('"', "\\\""))
        } else {
            item.text.clone()
        };
        self.filter_input.replace_range(item.replace, &insert);
        let needs_space = {
            let caret = self.filter_input.caret();
            match self.filter_input.text()[caret..].chars().next() {
                None => true,
                Some(c) if c.is_whitespace() || c == ')' => false,
                Some(_) => true,
            }
        };
        if needs_space {
            self.filter_input.insert(" ");
        }
        self.filter_popup_open = false;
        self.filter_error = None;
        cx.notify();
    }

    /// The filter bar's key handler. Enter applies, Esc closes,
    /// Backspace edits, and printable characters append. Unlike search
    /// the filter is not re-evaluated on every keystroke; the user
    /// commits with Enter.
    pub(super) fn on_filter_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.filter_open {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let extend = modifiers.shift;

        // Cmd/Ctrl+F switches straight to the search bar without needing
        // to close this one first.
        if key == "f" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
            self.switch_to_search(cx);
            return;
        }
        // Cmd/Ctrl+Shift+L switches to the Ask bar.
        if key == "l"
            && modifiers.secondary()
            && !modifiers.alt
            && !modifiers.function
            && modifiers.shift
        {
            self.switch_to_ask(cx);
            return;
        }

        // While the autocomplete popup is open it owns the navigation and
        // commit keys; Esc closes the popup before the bar itself.
        if self.filter_popup_open && !self.filter_suggestions.is_empty() {
            match key {
                "down" => {
                    self.filter_suggestion_selected = (self.filter_suggestion_selected + 1)
                        .min(self.filter_suggestions.len() - 1);
                    self.filter_suggestion_scroll
                        .scroll_to_item(self.filter_suggestion_selected);
                    cx.notify();
                    return;
                }
                "up" => {
                    self.filter_suggestion_selected =
                        self.filter_suggestion_selected.saturating_sub(1);
                    self.filter_suggestion_scroll
                        .scroll_to_item(self.filter_suggestion_selected);
                    cx.notify();
                    return;
                }
                "enter" | "tab" => {
                    self.accept_filter_suggestion(self.filter_suggestion_selected, cx);
                    return;
                }
                "escape" => {
                    self.filter_popup_open = false;
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }
        // Down with the popup closed opens it on the current slot.
        if key == "down" && !self.filter_suggestions.is_empty() {
            self.filter_popup_open = true;
            self.filter_suggestion_selected = 0;
            cx.notify();
            return;
        }

        if key == "escape" {
            self.close_filter(cx);
            return;
        }
        if key == "enter" {
            self.apply_filter_input(cx);
            return;
        }
        if key == "a" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
            self.filter_input.select_all();
            cx.notify();
            return;
        }

        // Word movement: Option+arrows on macOS, Ctrl+arrows elsewhere.
        // Cmd+Left/Right on macOS jump to the line ends.
        let word_move = if cfg!(target_os = "macos") {
            modifiers.alt
        } else {
            modifiers.control
        };
        let line_jump = cfg!(target_os = "macos") && modifiers.secondary();

        match key {
            "left" => {
                if line_jump {
                    self.filter_input.move_home(extend);
                } else if word_move {
                    self.filter_input.move_word_left(extend);
                } else {
                    self.filter_input.move_left(extend);
                }
                cx.notify();
                return;
            }
            "right" => {
                if line_jump {
                    self.filter_input.move_end(extend);
                } else if word_move {
                    self.filter_input.move_word_right(extend);
                } else {
                    self.filter_input.move_right(extend);
                }
                cx.notify();
                return;
            }
            "home" => {
                self.filter_input.move_home(extend);
                cx.notify();
                return;
            }
            "end" => {
                self.filter_input.move_end(extend);
                cx.notify();
                return;
            }
            "backspace" if !modifiers.control && !modifiers.alt => {
                self.filter_input.backspace();
                self.refresh_filter_suggestions();
                self.filter_popup_open = !self.filter_suggestions.is_empty();
                cx.notify();
                return;
            }
            "delete" if !modifiers.control && !modifiers.alt => {
                self.filter_input.delete();
                self.refresh_filter_suggestions();
                self.filter_popup_open = !self.filter_suggestions.is_empty();
                cx.notify();
                return;
            }
            _ => {}
        }

        if !modifiers.control && !modifiers.alt && !modifiers.function && !modifiers.secondary() {
            if let Some(ch) = super::super::input::printable_key_text(&event.keystroke) {
                self.filter_input.insert(ch);
                self.refresh_filter_suggestions();
                self.filter_popup_open = !self.filter_suggestions.is_empty();
                cx.notify();
            }
        }
    }

    /// Mouse press in the filter input: place the caret at the clicked
    /// byte index, extending the selection when Shift is held, and arm
    /// drag-select.
    pub(super) fn filter_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.filter_text_bounds else {
            return;
        };
        let index =
            input::text_index_at_point(window, self.filter_input.text(), bounds, event.position.x);
        self.filter_input.set_caret(index, event.modifiers.shift);
        self.filter_dragging = true;
        cx.notify();
    }

    /// Mouse drag in the filter input: extend the selection from the
    /// anchor to the byte index under the pointer. Only active between
    /// the press and release.
    pub(super) fn filter_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.filter_dragging || !event.dragging() {
            return;
        }
        let Some(bounds) = self.filter_text_bounds else {
            return;
        };
        let index =
            input::text_index_at_point(window, self.filter_input.text(), bounds, event.position.x);
        self.filter_input.set_caret(index, true);
        cx.notify();
    }

    pub(super) fn filter_mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.filter_dragging {
            self.filter_dragging = false;
            cx.notify();
        }
    }

    /// The autocomplete popup, positioned under the filter bar. Returned
    /// through `render_overlay` so it paints above the grid. `None` when
    /// the popup is closed or there is nothing to show.
    pub(super) fn render_filter_suggestions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let colors = theme::palette(cx);
        if !self.filter_popup_open || self.filter_suggestions.is_empty() {
            return None;
        }
        let selected = self.filter_suggestion_selected;
        let rows: Vec<AnyElement> = self
            .filter_suggestions
            .iter()
            .enumerate()
            .map(|(ix, item)| {
                div()
                    .id(("filter-suggestion", ix))
                    .debug_selector(move || format!("filter-suggestion-{ix}"))
                    .px_3()
                    .py_1()
                    .when(ix == selected, |s| s.bg(colors.selection))
                    .text_color(if ix == selected {
                        colors.strong_foreground
                    } else {
                        colors.foreground
                    })
                    .hover(|s| s.bg(colors.hover))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.accept_filter_suggestion(ix, cx)),
                    )
                    .child(item.text.clone())
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .id("filter-suggestions")
                .absolute()
                // Match the editor lane: band inset + label width + row gap.
                .top(gpui_kit::rems(2.25))
                .left(gpui_kit::rems(6.25))
                .w_80()
                .max_h_48()
                .overflow_y_scroll()
                .track_scroll(&self.filter_suggestion_scroll)
                .bg(colors.panel)
                .border_1()
                .border_color(colors.border_strong)
                .rounded_sm()
                .children(rows)
                .into_any_element(),
        )
    }

    /// Filter bar overlay (Cmd/Ctrl+L). A normal-flow child of the
    /// grid body, above the search bar, so it never overlaps the grid.
    /// Shows the input plus a status suffix (error / row count).
    pub(super) fn render_filter_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let colors = theme::palette(cx);
        if !self.filter_open {
            return None;
        }
        let (status_text, status_color) = self.filter_status_text(cx);

        let text_area = input::editable_text_area(
            window,
            cx,
            input::InputPresentation {
                edit: &self.filter_input,
                focus: &self.filter_focus_handle,
                placeholder: "Role == \"Parent\"",
                debug_id: "anvil-filter-input",
            },
            {
                let entity = cx.entity();
                move |bounds, app_cx| {
                    entity.update(app_cx, |view, _| {
                        view.filter_text_bounds = Some(bounds);
                    });
                }
            },
        )
        .on_mouse_down(
            gpui_kit::MouseButton::Left,
            cx.listener(Self::filter_mouse_down),
        )
        .on_mouse_move(cx.listener(Self::filter_mouse_move))
        .on_mouse_up(
            gpui_kit::MouseButton::Left,
            cx.listener(|this, _ev: &MouseUpEvent, _window, cx| this.filter_mouse_up(cx)),
        );

        Some(
            input::query_bar(colors)
                .min_w_0()
                .key_context(
                    if self.filter_popup_open && !self.filter_suggestions.is_empty() {
                        "Filter FilterAutocomplete"
                    } else {
                        "Filter"
                    },
                )
                .track_focus(&self.filter_focus_handle.clone().tab_stop(true))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if this.filter_focus_handle.is_focused(window)
                        || event.keystroke.key == "escape"
                    {
                        this.on_filter_key_down(event, window, cx);
                    }
                }))
                .child(input::query_label("Filter", cx))
                .child(input::query_field(
                    text_area,
                    &self.filter_focus_handle,
                    self.filter_error.is_some(),
                    window,
                    cx,
                ))
                .child(input::query_feedback(status_text, status_color))
                .child(input::query_rule(colors))
                .child(
                    Button::new("filter-apply")
                        .small()
                        .primary()
                        .icon(IconName::Check)
                        .label("Apply")
                        .tooltip("Apply expression · Enter")
                        .loading(self.filter_applied.is_some() && self.filter_rows.is_none())
                        .on_click(cx.listener(|this, _, _, cx| this.apply_filter_input(cx))),
                )
                .child(
                    Button::new("filter-clear")
                        .small()
                        .ghost()
                        .label("Clear")
                        .disabled(self.filter_applied.is_none() && self.filter_input.is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.clear_filter(cx))),
                )
                .child(
                    Button::new("filter-close")
                        .small()
                        .ghost()
                        .debug_selector(|| "filter-close".to_string())
                        .icon(IconName::Close)
                        .accessibility_label("Close Filter")
                        .tooltip("Close Filter · Esc · Keeps the applied filter")
                        .on_click(cx.listener(|this, _, _, cx| this.close_filter(cx))),
                )
                .into_any_element(),
        )
    }

    /// Status suffix for the filter bar: a parse error, a "filtering..."
    /// note, the row count, or the shortcut hint.
    pub(super) fn filter_status_text(&self, cx: &gpui_kit::App) -> (String, gpui_kit::Hsla) {
        let colors = theme::palette(cx);
        if let Some(error) = &self.filter_error {
            return (error.clone(), colors.danger);
        }
        match (&self.filter_applied, &self.filter_rows) {
            (Some(_), Some(rows)) if rows.is_empty() => {
                ("no rows match".to_string(), colors.warning)
            }
            (Some(_), Some(rows)) => (
                format!(
                    "{} of {} rows",
                    format_count(rows.len() as u64),
                    format_count(self.model.as_ref().map(|m| m.row_count()).unwrap_or(0))
                ),
                colors.muted_foreground,
            ),
            (Some(_), None) => ("filtering...".to_string(), colors.muted_foreground),
            (None, _) => (
                "Enter applies, Tab completes".to_string(),
                colors.muted_foreground,
            ),
        }
    }
}

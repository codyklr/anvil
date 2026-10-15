//! Search bar: state transitions, background search, rendering, and
//! mouse/keyboard editing.
//!
//! The search state itself stays on `MainView`; this module holds the
//! search half of its inherent methods plus the column-label helper
//! used by the scope chip. The shared editable-field rendering lives
//! in `super::input`.

use gpui_kit::component::{
    button::{Button, ButtonVariants},
    Disableable, IconName, Selectable, Sizable,
};

use super::*;

impl MainView {
    /// Open the search bar overlay and kick off a search for whatever
    /// query was last typed (empty by default → 0 matches). The bar
    /// takes keyboard focus so subsequent keystrokes go into the input
    /// rather than the grid.
    pub(super) fn open_search(&mut self, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search_matches.clear();
        self.search_active = -1;
        self.focus_search_on_next_frame = true;
        self.kick_search(cx);
        cx.notify();
    }

    /// Close the search bar and clear all search state. The grid keeps
    /// its current selection; the search rectangle stops being
    /// highlighted.
    pub(super) fn close_search(&mut self) {
        // Snapshot first so the active tab's persisted search state
        // is consistent - the live fields are about to be reset to
        // "no search", which is what the snapshot now records.
        self.snapshot_active();
        self.search_open = false;
        self.search_query.clear();
        self.search_matches.clear();
        self.search_active = -1;
        self.search_task = None;
        // search_generation is bumped implicitly on the next open / edit.
        // The column scope and case-sensitivity persist so the
        // "click a header, then Cmd/Ctrl+F" flow survives Esc; the
        // scoped header stays highlighted while the bar is closed.
    }

    /// Toggle the column scope for search. Clicking a scoped header
    /// again clears the scope, so the same gesture both sets and
    /// clears it (including while the bar is closed). Re-runs an open
    /// search so matches and highlighting follow the new scope.
    pub(super) fn toggle_search_column(&mut self, column: usize, cx: &mut Context<Self>) {
        self.search_column = if self.search_column == Some(column) {
            None
        } else {
            Some(column)
        };
        if self.search_open {
            self.kick_search(cx);
        }
        // Keep keyboard navigation on the grid after a header click.
        self.focus_grid_on_next_frame = true;
        cx.notify();
    }

    /// Clear the column scope (the search chip's ×). Re-runs an open
    /// search so it widens back to every column.
    pub(super) fn clear_search_column(&mut self, cx: &mut Context<Self>) {
        if self.search_column.is_none() {
            return;
        }
        self.search_column = None;
        if self.search_open {
            self.kick_search(cx);
        }
        cx.notify();
    }

    /// Flip the active tab's search between case-insensitive (default)
    /// and byte-exact (`Aa` in the bar). Re-runs an open search so the
    /// match list and highlighting stay in sync with the toggle.
    pub(super) fn toggle_search_case_sensitive(&mut self, cx: &mut Context<Self>) {
        self.search_case_sensitive = !self.search_case_sensitive;
        if self.search_open {
            self.kick_search(cx);
        }
        cx.notify();
    }

    /// The `SearchOptions` for the active tab. Single source of truth
    /// for both the background search and the grid's highlight pass,
    /// so the two cannot drift apart.
    pub(super) fn search_options(&self) -> SearchOptions {
        SearchOptions {
            case_sensitive: self.search_case_sensitive,
            column: self.search_column,
            max_matches: None,
            encoding: self.display_encoding,
        }
    }

    /// Start (or restart) the background search for `self.search_query`.
    /// Bumps `search_generation` and drops the previous task. The task
    /// captures the generation so a stale result is silently discarded.
    pub(super) fn kick_search(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            self.search_matches.clear();
            self.search_active = -1;
            return;
        };
        let needle = self.search_query.text().to_string();
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        let doc = model.doc.clone();
        let options = self.search_options();

        // Reset visible state while the search is in flight so the user
        // sees an empty match list rather than the previous run's.
        self.search_matches.clear();
        self.search_active = -1;

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let matches = executor
                .spawn(async move { search_records(&doc, &needle, &options) })
                .await;
            this.update(cx, |view, cx| {
                if view.search_generation != generation {
                    return;
                }
                view.apply_search_result(matches, cx);
            })
            .ok();
        });
        self.search_task = Some(task);
    }

    /// Apply the result of an in-flight search, ordered by record
    /// index. Picks the first match as the active one (or the closest
    /// after the cursor, if there is one).
    pub(super) fn apply_search_result(&mut self, matches: Vec<u64>, cx: &mut Context<Self>) {
        self.search_matches = matches;
        self.search_active = if self.search_matches.is_empty() {
            -1
        } else {
            0
        };
        if self.search_active >= 0 {
            self.jump_to_active_match(cx);
        }
        cx.notify();
    }

    /// Move the active match by `direction` (1 = next, -1 = previous)
    /// and scroll the grid to it. A no-op when no search is active.
    pub(super) fn advance_match(&mut self, direction: i8, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }
        let n = self.search_matches.len() as i64;
        let next = if self.search_active < 0 {
            if direction > 0 {
                0
            } else {
                n - 1
            }
        } else {
            // i64 wrap-safe cycle.
            let raw = self.search_active + direction as i64;
            ((raw % n) + n) % n
        };
        self.search_active = next;
        self.jump_to_active_match(cx);
    }

    /// Move the selection cursor to the active match's row and scroll
    /// the list so the row is visible.
    pub(super) fn jump_to_active_match(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let Some(row) = self
            .search_matches
            .get(self.search_active as usize)
            .copied()
        else {
            return;
        };
        // Land on the scoped column when set, otherwise the first column
        // whose cell contains the needle, otherwise whatever column the
        // cursor was already on.
        let col = self
            .search_column
            .or_else(|| self.first_match_column(row))
            .unwrap_or_else(|| self.selection.cursor().map(|c| c.column_index).unwrap_or(0));
        let col = col.min(model.doc.column_count().saturating_sub(1));
        // Bring the match on screen in both axes. The selection is in
        // display space, so map the record index to its display position
        // before moving the cursor and scrolling; the matching column may
        // be far to the left or right of the current viewport.
        let position = self.display_position(row);
        self.selection.set(CellPos::new(position as u64, col));
        self.v_scroll
            .scroll_to_item_strict(position, ScrollStrategy::Top);
        self.ensure_column_visible(col, cx);
        cx.notify();
    }

    /// The column to land on for a whole-record match: the scoped column
    /// when one is set, otherwise the first column whose cell contains
    /// the needle. `None` when the needle is not found in the row (e.g. a
    /// stale match list).
    fn first_match_column(&self, row: u64) -> Option<usize> {
        let model = self.model.as_ref()?;
        let needle = self.search_query.text();
        if needle.is_empty() {
            return None;
        }
        let case_sensitive = self.search_case_sensitive;
        for column in 0..model.doc.column_count() {
            if let Some(cell) = model.doc.cell(row, ColumnIndex(column)) {
                if !find_ranges(cell.as_bytes(), needle.as_bytes(), case_sensitive).is_empty() {
                    return Some(column);
                }
            }
        }
        None
    }

    /// The search bar's keyboard handler. Char keys append to the
    /// query; backspace deletes the last byte; Enter / Shift+Enter
    /// cycle matches; Esc closes the bar. Anything else falls through
    /// (and is therefore ignored).
    pub(super) fn on_search_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.search_open {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;

        // Cmd/Ctrl+L switches straight to the filter bar, and
        // Cmd/Ctrl+Shift+L to the Ask bar, without closing first.
        if key == "l" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
            if modifiers.shift {
                self.switch_to_ask(cx);
            } else {
                self.switch_to_filter(cx);
            }
            return;
        }

        // Escape closes the bar and returns focus to the grid. This
        // re-uses `Action::Escape` so the binding table stays the
        // single source of truth, even though we are *not* the grid.
        if key == "escape" {
            self.close_search();
            self.focus_grid_on_next_frame = true;
            cx.notify();
            return;
        }

        // Enter / Shift+Enter cycle through matches without leaving the
        // search bar.
        if key == "enter" {
            let direction: i8 = if modifiers.shift { -1 } else { 1 };
            self.advance_match(direction, cx);
            return;
        }

        // Cmd/Ctrl+A selects the whole query.
        if key == "a" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
            self.search_query.select_all();
            cx.notify();
            return;
        }

        let extend = modifiers.shift;
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
                    self.search_query.move_home(extend);
                } else if word_move {
                    self.search_query.move_word_left(extend);
                } else {
                    self.search_query.move_left(extend);
                }
                cx.notify();
                return;
            }
            "right" => {
                if line_jump {
                    self.search_query.move_end(extend);
                } else if word_move {
                    self.search_query.move_word_right(extend);
                } else {
                    self.search_query.move_right(extend);
                }
                cx.notify();
                return;
            }
            "home" => {
                self.search_query.move_home(extend);
                cx.notify();
                return;
            }
            "end" => {
                self.search_query.move_end(extend);
                cx.notify();
                return;
            }
            "backspace" if !modifiers.control && !modifiers.alt => {
                self.search_query.backspace();
                self.kick_search(cx);
                cx.notify();
                return;
            }
            "delete" if !modifiers.control && !modifiers.alt => {
                self.search_query.delete();
                self.kick_search(cx);
                cx.notify();
                return;
            }
            _ => {}
        }

        // Plain printable characters replace the selection / insert at
        // the caret. We require no modifiers so we don't swallow
        // Cmd+F (already handled by the grid binding), Option+E (an
        // accented character), etc.
        if !modifiers.control && !modifiers.alt && !modifiers.function && !modifiers.secondary() {
            if let Some(ch) = super::super::input::printable_key_text(&event.keystroke) {
                self.search_query.insert(ch);
                self.kick_search(cx);
                cx.notify();
            }
        }
    }

    /// Mouse press in the search input: place the caret at the clicked
    /// byte index, extending the selection when Shift is held, and arm
    /// drag-select.
    pub(super) fn search_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.search_text_bounds else {
            return;
        };
        let index =
            input::text_index_at_point(window, self.search_query.text(), bounds, event.position.x);
        self.search_query.set_caret(index, event.modifiers.shift);
        self.search_dragging = true;
        cx.notify();
    }

    /// Mouse drag in the search input: extend the selection from the
    /// anchor to the byte index under the pointer.
    pub(super) fn search_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.search_dragging || !event.dragging() {
            return;
        }
        let Some(bounds) = self.search_text_bounds else {
            return;
        };
        let index =
            input::text_index_at_point(window, self.search_query.text(), bounds, event.position.x);
        self.search_query.set_caret(index, true);
        cx.notify();
    }

    pub(super) fn search_mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.search_dragging {
            self.search_dragging = false;
            cx.notify();
        }
    }

    // ---- filter ----------------------------------------------------------

    /// Search bar overlay (Cmd/Ctrl+F). Sits as a normal-flow child
    /// of the center area (above the grid, below the toolbar) so the
    /// grid never slides under it; this keeps the active match's
    /// highlight fully visible even when the match is at row 0. The
    /// bar carries its own `key_context("Search")` and `track_focus`
    /// so characters typed while it is open route through
    /// `on_search_key_down` rather than the grid handler.
    pub(super) fn render_search_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let colors = theme::palette(cx);
        if !self.search_open {
            return None;
        }
        let (status_text, status_color) = self.search_status_text(cx);
        let case_active = self.search_case_sensitive;
        let scope_label = self.search_column.map(|col| {
            let headers = self
                .model
                .as_ref()
                .map(|m| m.doc.meta.headers.as_slice())
                .unwrap_or(&[]);
            search_column_label(headers, col)
        });

        let editor = input::editable_text_area(
            window,
            cx,
            input::InputPresentation {
                edit: &self.search_query,
                focus: &self.search_focus_handle,
                placeholder: "Search values…",
                debug_id: "anvil-search-input",
            },
            {
                let entity = cx.entity();
                move |bounds, app_cx| {
                    entity.update(app_cx, |view, _| view.search_text_bounds = Some(bounds));
                }
            },
        )
        .on_mouse_down(
            gpui_kit::MouseButton::Left,
            cx.listener(Self::search_mouse_down),
        )
        .on_mouse_move(cx.listener(Self::search_mouse_move))
        .on_mouse_up(
            gpui_kit::MouseButton::Left,
            cx.listener(|this, _: &MouseUpEvent, _, cx| this.search_mouse_up(cx)),
        );

        Some(
            input::query_bar(colors)
                .min_w_0()
                .key_context("Search")
                .track_focus(&self.search_focus_handle.clone().tab_stop(true))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    // Buttons own their keyboard activation; only the editor edits text.
                    if this.search_focus_handle.is_focused(window)
                        || event.keystroke.key == "escape"
                    {
                        this.on_search_key_down(event, window, cx);
                    }
                }))
                .child(input::query_label("Find", cx))
                .child(input::query_field(
                    editor,
                    &self.search_focus_handle,
                    false,
                    window,
                    cx,
                ))
                .child(
                    Button::new("search-case-toggle")
                        .small()
                        .ghost()
                        .icon(IconName::CaseSensitive)
                        .accessibility_label("Match case")
                        .tooltip("Match case exactly")
                        .selected(case_active)
                        .toggled(case_active)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.toggle_search_case_sensitive(cx)),
                        ),
                )
                .child(
                    Button::new("search-previous")
                        .small()
                        .ghost()
                        .icon(IconName::ChevronUp)
                        .accessibility_label("Previous match")
                        .tooltip("Previous match · Shift+Enter / Shift+F3")
                        .disabled(self.search_matches.is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.advance_match(-1, cx))),
                )
                .child(
                    Button::new("search-next")
                        .small()
                        .ghost()
                        .icon(IconName::ChevronDown)
                        .accessibility_label("Next match")
                        .tooltip("Next match · Enter / F3")
                        .disabled(self.search_matches.is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.advance_match(1, cx))),
                )
                // The match counter sits with the navigation controls it
                // drives, not on a row of its own far from them.
                .child(input::query_feedback(status_text, status_color))
                .child(input::query_rule(colors))
                .when_some(scope_label, |row, label| {
                    row.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .flex_shrink_0()
                            .child(div().text_xs().text_color(colors.foreground).child(label))
                            .child(
                                Button::new("search-scope-clear")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .accessibility_label("Search all columns")
                                    .tooltip("Remove column scope")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.clear_search_column(cx)),
                                    ),
                            ),
                    )
                })
                .child(
                    Button::new("search-close")
                        .small()
                        .ghost()
                        .debug_selector(|| "search-close".to_string())
                        .icon(IconName::Close)
                        .accessibility_label("Close Find")
                        .tooltip("Close Find · Esc")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_search();
                            this.focus_grid_on_next_frame = true;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    /// Status-bar text + colour for the search bar: "N matches", "no
    /// matches", or "searching…" depending on whether a task is in
    /// flight.
    pub(super) fn search_status_text(&self, cx: &gpui_kit::App) -> (String, gpui_kit::Hsla) {
        let colors = theme::palette(cx);
        let basis = match self.display_encoding {
            anvil_core::DisplayEncoding::Utf8 => "file bytes",
            anvil_core::DisplayEncoding::Windows1252 => "decoded text",
        };
        if self.search_query.is_empty() {
            return (
                if self.search_column.is_some() {
                    format!("scoped to one column, {basis}")
                } else {
                    format!("searching {basis}")
                },
                colors.muted_foreground,
            );
        }
        if !self.search_query.is_empty() && self.search_matches.is_empty() {
            // The empty match list could be either "still searching" or
            // "no matches". We treat an in-flight search task (or a
            // generation mismatch) as "searching…".
            if self.search_task.is_some() {
                return (format!("searching {basis}…"), colors.muted_foreground);
            }
            return (format!("no matches in {basis}"), colors.warning);
        }
        let total = self.search_matches.len();
        let active = if self.search_active >= 0 {
            self.search_active + 1
        } else {
            0
        };
        (
            format!("{active} of {total}, {basis}"),
            colors.muted_foreground,
        )
    }
}

/// Human label for a search column: the header name when present
/// and non-empty, otherwise the generated `col0`/`col1` label. Used by
/// the search bar's scope chip so the user sees the same name as in the
/// grid header.
pub(super) fn search_column_label(headers: &[String], column: usize) -> String {
    anvil_core::column_label(headers, column)
}

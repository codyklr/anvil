//! Keyboard command layer.
//!
//! This module is the single place that maps a key chord to an intent.
//! It is deliberately free of GPUI view state: `action_for` takes a
//! `Keystroke` and returns an [`Action`], nothing more. `MainView`
//! executes the returned action (`view.rs::run_action`). Keeping the
//! binding table pure means it can be unit-tested without a window, and
//! means the keyboard surface can grow (search, filter, go-to-row,
//! select-all) by adding one match arm and one action variant instead of
//! threading more `if key == ...` checks through the view.
//!
//! This is intentionally *not* GPUI's typed-action system (`actions!` +
//! `cx.bind_keys`). That layer would enable user-rebindable shortcuts
//! and menu generation, but it doubles the action count for shifted
//! variants and is a larger change; it can be layered on top of this
//! module later without changing callers.

use gpui::Keystroke;

/// How many rows a Page Up / Page Down jump moves the cursor. Tied to a
/// conservative estimate of the visible row count so the grid does not
/// have to be measured on every keypress.
pub(super) const PAGE_STEP: i64 = 20;

/// A logical keyboard intent. The view executes these; they carry no
/// GPUI types so the binding table stays testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    /// Move the selection cursor by `(row_delta, column_delta)`. The
    /// existing `Selection::move_cursor` clamps at the document bounds,
    /// so `i64::MIN` / `i64::MAX` are used for Home / End.
    MoveCursor {
        row_delta: i64,
        column_delta: i64,
        extend: bool,
    },
    /// Move the cursor to the adjacent cell in row-major order,
    /// wrapping across rows (Tab / Shift-Tab).
    NextCell { forward: bool },
    /// Copy the current selection to the clipboard.
    Copy,
    /// Clear the selection, or close the archive picker if already
    /// empty.
    Escape,
    /// Open the search bar (Cmd/Ctrl+F). When the bar is already
    /// open, the binding is a no-op so the grid does not lose focus.
    OpenSearch,
    /// Jump to the next or previous search match. `direction == -1`
    /// is previous, `+1` is next. The actual navigation depends on
    /// the current match list.
    NextMatch { direction: i8 },
    /// Show or hide the file-manager side panel (Cmd/Ctrl+B).
    ToggleFilePanel,
    /// Open the filter bar (Cmd/Ctrl+L). No-op when already open.
    OpenFilter,
    /// Open the natural-language Ask bar (Cmd/Ctrl+Shift+L).
    OpenAsk,
    /// Show or hide the column-inspector panel (Cmd/Ctrl+I).
    ToggleInspector,
    /// Show or hide the diagnostics panel (Cmd/Ctrl+D).
    ToggleDiagnostics,
    /// Select the whole view: anchor at the first cell, cursor at the
    /// last (Cmd/Ctrl+A). With an active filter or sort the grid and
    /// copy honour the visible display order.
    SelectAll,
    /// Open the go-to-row input bar (Cmd/Ctrl+G).
    OpenGoToRow,
    /// Open a file via the same path as the toolbar **Open…** button
    /// (Cmd/Ctrl+O).
    OpenFile,
}

/// Translate a keystroke into an [`Action`], or `None` when the chord is
/// not bound. `modifiers.secondary()` is Cmd on macOS and Ctrl
/// elsewhere.
pub(super) fn action_for(keystroke: &Keystroke) -> Option<Action> {
    let key = keystroke.key.as_str();
    let modifiers = keystroke.modifiers;

    if key == "c" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return Some(Action::Copy);
    }

    if key == "f" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return Some(Action::OpenSearch);
    }

    if key == "b" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return Some(Action::ToggleFilePanel);
    }

    if key == "l" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return Some(if modifiers.shift {
            Action::OpenAsk
        } else {
            Action::OpenFilter
        });
    }

    if key == "i" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return Some(Action::ToggleInspector);
    }

    if key == "d" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return Some(Action::ToggleDiagnostics);
    }

    if key == "a" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return Some(Action::SelectAll);
    }

    if key == "g" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return Some(Action::OpenGoToRow);
    }

    if key == "o" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return Some(Action::OpenFile);
    }

    // Cmd/Ctrl+Home / Cmd/Ctrl+End jump to the first / last row while
    // keeping the current column. Plain Home / End still jump to the
    // first / last column below.
    if (key == "home" || key == "end")
        && modifiers.secondary()
        && !modifiers.alt
        && !modifiers.function
    {
        return Some(Action::MoveCursor {
            row_delta: if key == "home" { i64::MIN } else { i64::MAX },
            column_delta: 0,
            extend: modifiers.shift,
        });
    }

    if key == "tab" {
        return Some(Action::NextCell {
            forward: !modifiers.shift,
        });
    }

    if key == "escape" {
        return Some(Action::Escape);
    }

    let (row_delta, column_delta) = match key {
        "up" => (-1, 0),
        "down" => (1, 0),
        "left" => (0, -1),
        "right" => (0, 1),
        "pageup" => (-PAGE_STEP, 0),
        "pagedown" => (PAGE_STEP, 0),
        "home" => (0, i64::MIN),
        "end" => (0, i64::MAX),
        "f3" => {
            // F3 / Shift-F3 jumps between search matches. The actual
            // navigation is performed by `MainView::run_action`, which
            // knows whether a search is in progress.
            let direction = if modifiers.shift { -1 } else { 1 };
            return Some(Action::NextMatch { direction });
        }
        _ => return None,
    };

    Some(Action::MoveCursor {
        row_delta,
        column_delta,
        extend: modifiers.shift,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(chord: &str) -> Option<Action> {
        action_for(&Keystroke::parse(chord).expect("valid keystroke"))
    }

    /// Cmd on macOS, Ctrl elsewhere - whichever `secondary()` matches.
    fn copy_chord() -> &'static str {
        if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        }
    }

    #[test]
    fn copy_chord_maps_to_copy() {
        assert_eq!(act(copy_chord()), Some(Action::Copy));
    }

    #[test]
    fn plain_c_is_not_copy() {
        assert_eq!(act("c"), None);
    }

    #[test]
    fn alt_c_is_not_copy() {
        // Alt (option) must not be swallowed as Copy.
        assert_eq!(act("alt-c"), None);
    }

    #[test]
    fn arrow_keys_map_to_single_cell_moves() {
        assert_eq!(
            act("up"),
            Some(Action::MoveCursor {
                row_delta: -1,
                column_delta: 0,
                extend: false,
            })
        );
        assert_eq!(
            act("down"),
            Some(Action::MoveCursor {
                row_delta: 1,
                column_delta: 0,
                extend: false,
            })
        );
        assert_eq!(
            act("left"),
            Some(Action::MoveCursor {
                row_delta: 0,
                column_delta: -1,
                extend: false,
            })
        );
        assert_eq!(
            act("right"),
            Some(Action::MoveCursor {
                row_delta: 0,
                column_delta: 1,
                extend: false,
            })
        );
    }

    #[test]
    fn shift_arrow_extends() {
        assert_eq!(
            act("shift-down"),
            Some(Action::MoveCursor {
                row_delta: 1,
                column_delta: 0,
                extend: true,
            })
        );
    }

    #[test]
    fn page_keys_step_by_page_step() {
        assert_eq!(
            act("pageup"),
            Some(Action::MoveCursor {
                row_delta: -PAGE_STEP,
                column_delta: 0,
                extend: false,
            })
        );
        assert_eq!(
            act("pagedown"),
            Some(Action::MoveCursor {
                row_delta: PAGE_STEP,
                column_delta: 0,
                extend: false,
            })
        );
    }

    #[test]
    fn home_and_end_jump_to_column_extremes() {
        assert_eq!(
            act("home"),
            Some(Action::MoveCursor {
                row_delta: 0,
                column_delta: i64::MIN,
                extend: false,
            })
        );
        assert_eq!(
            act("end"),
            Some(Action::MoveCursor {
                row_delta: 0,
                column_delta: i64::MAX,
                extend: false,
            })
        );
    }

    #[test]
    fn cmd_home_and_cmd_end_jump_to_row_extremes() {
        let home = if cfg!(target_os = "macos") {
            "cmd-home"
        } else {
            "ctrl-home"
        };
        let end = if cfg!(target_os = "macos") {
            "cmd-end"
        } else {
            "ctrl-end"
        };
        assert_eq!(
            act(home),
            Some(Action::MoveCursor {
                row_delta: i64::MIN,
                column_delta: 0,
                extend: false,
            })
        );
        assert_eq!(
            act(end),
            Some(Action::MoveCursor {
                row_delta: i64::MAX,
                column_delta: 0,
                extend: false,
            })
        );
    }

    #[test]
    fn cmd_shift_home_extends_to_first_row() {
        let chord = if cfg!(target_os = "macos") {
            "cmd-shift-home"
        } else {
            "ctrl-shift-home"
        };
        assert_eq!(
            act(chord),
            Some(Action::MoveCursor {
                row_delta: i64::MIN,
                column_delta: 0,
                extend: true,
            })
        );
    }

    #[test]
    fn tab_and_shift_tab_move_adjacent_cells() {
        assert_eq!(act("tab"), Some(Action::NextCell { forward: true }));
        assert_eq!(act("shift-tab"), Some(Action::NextCell { forward: false }));
    }

    #[test]
    fn escape_maps_to_escape() {
        assert_eq!(act("escape"), Some(Action::Escape));
    }

    #[test]
    fn unbound_key_returns_none() {
        assert_eq!(act("a"), None);
        assert_eq!(act("f5"), None);
    }

    #[test]
    fn open_search_chord_maps_to_open_search() {
        let chord = if cfg!(target_os = "macos") {
            "cmd-f"
        } else {
            "ctrl-f"
        };
        assert_eq!(act(chord), Some(Action::OpenSearch));
    }

    #[test]
    fn plain_f_is_not_open_search() {
        assert_eq!(act("f"), None);
    }

    #[test]
    fn alt_f_is_not_open_search() {
        assert_eq!(act("alt-f"), None);
    }

    #[test]
    fn f3_jumps_to_next_match() {
        assert_eq!(act("f3"), Some(Action::NextMatch { direction: 1 }));
    }

    #[test]
    fn shift_f3_jumps_to_previous_match() {
        assert_eq!(act("shift-f3"), Some(Action::NextMatch { direction: -1 }));
    }

    #[test]
    fn toggle_file_panel_chord() {
        let chord = if cfg!(target_os = "macos") {
            "cmd-b"
        } else {
            "ctrl-b"
        };
        assert_eq!(act(chord), Some(Action::ToggleFilePanel));
    }

    #[test]
    fn plain_b_is_not_toggle_file_panel() {
        assert_eq!(act("b"), None);
    }

    #[test]
    fn open_filter_chord() {
        let chord = if cfg!(target_os = "macos") {
            "cmd-l"
        } else {
            "ctrl-l"
        };
        assert_eq!(act(chord), Some(Action::OpenFilter));
    }

    #[test]
    fn plain_l_is_not_open_filter() {
        assert_eq!(act("l"), None);
    }

    #[test]
    fn toggle_inspector_chord() {
        let chord = if cfg!(target_os = "macos") {
            "cmd-i"
        } else {
            "ctrl-i"
        };
        assert_eq!(act(chord), Some(Action::ToggleInspector));
    }

    #[test]
    fn plain_i_is_not_toggle_inspector() {
        assert_eq!(act("i"), None);
    }

    #[test]
    fn toggle_diagnostics_chord() {
        let chord = if cfg!(target_os = "macos") {
            "cmd-d"
        } else {
            "ctrl-d"
        };
        assert_eq!(act(chord), Some(Action::ToggleDiagnostics));
    }

    #[test]
    fn plain_d_is_not_toggle_diagnostics() {
        assert_eq!(act("d"), None);
    }

    #[test]
    fn alt_d_is_not_toggle_diagnostics() {
        assert_eq!(act("alt-d"), None);
    }

    #[test]
    fn select_all_chord() {
        let chord = if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        };
        assert_eq!(act(chord), Some(Action::SelectAll));
    }

    #[test]
    fn plain_a_is_not_select_all() {
        assert_eq!(act("a"), None);
    }

    #[test]
    fn alt_a_is_not_select_all() {
        assert_eq!(act("alt-a"), None);
    }

    #[test]
    fn go_to_row_chord() {
        let chord = if cfg!(target_os = "macos") {
            "cmd-g"
        } else {
            "ctrl-g"
        };
        assert_eq!(act(chord), Some(Action::OpenGoToRow));
    }

    #[test]
    fn plain_g_is_not_go_to_row() {
        assert_eq!(act("g"), None);
    }

    #[test]
    fn open_file_chord() {
        let chord = if cfg!(target_os = "macos") {
            "cmd-o"
        } else {
            "ctrl-o"
        };
        assert_eq!(act(chord), Some(Action::OpenFile));
    }

    #[test]
    fn plain_o_is_not_open_file() {
        assert_eq!(act("o"), None);
    }
}

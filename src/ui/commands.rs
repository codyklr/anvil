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

    /// Cmd on macOS, Ctrl elsewhere — whichever `secondary()` matches.
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
}

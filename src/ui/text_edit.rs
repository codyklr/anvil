//! A small editable-text buffer for the search / filter inputs.
//!
//! GPUI 0.2.2 has no `TextInput` element, so the bars render and edit a
//! plain string by hand. This type is the GPUI-free state machine behind
//! that: it owns the text plus a caret and an anchor (the selection
//! origin). It models exactly what a single-line field needs - insert,
//! delete, caret movement, and range selection - and is unit-testable
//! without a window, mirroring `anvil_core::Selection` for the grid.
//!
//! Invariants
//! ----------
//! * `caret` and `anchor` are byte offsets into `text`, always on a
//!   UTF-8 character boundary (clamped on every mutation).
//! * `anchor == caret` means "no selection". Otherwise the selection is
//!   the inclusive-exclusive range between them, in either direction.
//! * Mutating operations (`insert`, `backspace`, `delete`) collapse the
//!   selection and leave the caret after the edit.
//!
//! The type carries no pixels and no GPUI types; hit-testing a pointer
//! to a byte index lives in the view, which owns the text layout.

use std::ops::Range;

/// Single-line editable text with a caret and an anchor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextEdit {
    text: String,
    /// Caret byte offset (the moving end of the selection).
    caret: usize,
    /// Selection anchor byte offset (the fixed end). Equal to `caret`
    /// when there is no selection.
    anchor: usize,
}

impl TextEdit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build an editor around `text` with the caret at the end.
    pub fn from_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            caret: end,
            anchor: end,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Clear the text and place the caret at the start.
    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
    }

    /// Replace the contents and place the caret at the end.
    #[allow(dead_code)] // exercised by tests and the From impls
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.caret = self.text.len();
        self.anchor = self.caret;
    }

    /// The selected byte range. Empty when there is no selection.
    pub fn selection(&self) -> Range<usize> {
        let start = self.anchor.min(self.caret);
        let end = self.anchor.max(self.caret);
        start..end
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    #[allow(dead_code)] // exercised by unit tests
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    /// Move the caret to `index` (clamped to a char boundary). When
    /// `extend` is set the anchor is kept so the selection grows from
    /// where it started; otherwise the selection collapses.
    pub fn set_caret(&mut self, index: usize, extend: bool) {
        let index = clamp_boundary(&self.text, index);
        self.caret = index;
        if !extend {
            self.anchor = index;
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Left arrow. With an existing selection and no `extend`, collapses
    /// to the selection's start rather than moving one more character.
    pub fn move_left(&mut self, extend: bool) {
        let target = if self.has_selection() && !extend {
            self.selection().start
        } else {
            prev_boundary(&self.text, self.caret)
        };
        self.set_caret(target, extend);
    }

    /// Right arrow. With an existing selection and no `extend`, collapses
    /// to the selection's end.
    pub fn move_right(&mut self, extend: bool) {
        let target = if self.has_selection() && !extend {
            self.selection().end
        } else {
            next_boundary(&self.text, self.caret)
        };
        self.set_caret(target, extend);
    }

    pub fn move_home(&mut self, extend: bool) {
        self.set_caret(0, extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        let end = self.text.len();
        self.set_caret(end, extend);
    }

    pub fn move_word_left(&mut self, extend: bool) {
        let target = word_left(&self.text, self.caret);
        self.set_caret(target, extend);
    }

    pub fn move_word_right(&mut self, extend: bool) {
        let target = word_right(&self.text, self.caret);
        self.set_caret(target, extend);
    }

    /// Insert `s`, replacing the selection if any, and leave the caret
    /// after the inserted text.
    pub fn insert(&mut self, s: &str) {
        let selection = self.selection();
        self.text.replace_range(selection.clone(), s);
        self.caret = selection.start + s.len();
        self.anchor = self.caret;
    }

    /// Delete the selection, or the character before the caret. Returns
    /// `true` when the text changed.
    pub fn backspace(&mut self) -> bool {
        let selection = self.selection();
        if !selection.is_empty() {
            self.text.replace_range(selection.clone(), "");
            self.caret = selection.start;
            self.anchor = self.caret;
            return true;
        }
        if self.caret == 0 {
            return false;
        }
        let start = prev_boundary(&self.text, self.caret);
        self.text.replace_range(start..self.caret, "");
        self.caret = start;
        self.anchor = start;
        true
    }

    /// Delete the selection, or the character after the caret. Returns
    /// `true` when the text changed.
    pub fn delete(&mut self) -> bool {
        let selection = self.selection();
        if !selection.is_empty() {
            self.text.replace_range(selection.clone(), "");
            self.caret = selection.start;
            self.anchor = self.caret;
            return true;
        }
        if self.caret >= self.text.len() {
            return false;
        }
        let end = next_boundary(&self.text, self.caret);
        self.text.replace_range(self.caret..end, "");
        true
    }

    /// Replace `range` with `text`, leaving the caret and anchor just
    /// after the inserted text. The range is clamped to char boundaries.
    /// Used by autocomplete to replace the partial token under the caret.
    pub fn replace_range(&mut self, range: Range<usize>, text: &str) {
        let start = clamp_boundary(&self.text, range.start);
        let end = clamp_boundary(&self.text, range.end.max(start));
        self.text.replace_range(start..end, text);
        self.caret = start + text.len();
        self.anchor = self.caret;
    }
}

impl From<&str> for TextEdit {
    fn from(text: &str) -> Self {
        Self::from_text(text)
    }
}

impl From<String> for TextEdit {
    fn from(text: String) -> Self {
        Self::from_text(text)
    }
}

/// Clamp `index` to `text.len()` and back up to the nearest char
/// boundary so the caret can never split a multi-byte character.
fn clamp_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// The char boundary immediately before `index` (or 0).
fn prev_boundary(text: &str, index: usize) -> usize {
    if index == 0 {
        return 0;
    }
    let mut i = index - 1;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The char boundary immediately after `index` (or `text.len()`).
fn next_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut i = index + 1;
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Skip back over trailing whitespace, then over the preceding word.
fn word_left(text: &str, index: usize) -> usize {
    let mut i = clamp_boundary(text, index);
    while i > 0 {
        let prev = prev_boundary(text, i);
        if text[prev..i].chars().all(char::is_whitespace) {
            i = prev;
        } else {
            break;
        }
    }
    while i > 0 {
        let prev = prev_boundary(text, i);
        if text[prev..i].chars().any(char::is_whitespace) {
            break;
        }
        i = prev;
    }
    i
}

/// Skip forward over leading whitespace, then to the end of the word.
fn word_right(text: &str, index: usize) -> usize {
    let mut i = clamp_boundary(text, index);
    while i < text.len() {
        let next = next_boundary(text, i);
        if text[i..next].chars().all(char::is_whitespace) {
            i = next;
        } else {
            break;
        }
    }
    while i < text.len() {
        let next = next_boundary(text, i);
        if text[i..next].chars().any(char::is_whitespace) {
            break;
        }
        i = next;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_editor_is_empty_at_origin() {
        let edit = TextEdit::new();
        assert!(edit.is_empty());
        assert_eq!(edit.caret(), 0);
        assert!(!edit.has_selection());
        assert_eq!(edit.selection(), 0..0);
    }

    #[test]
    fn from_text_places_caret_at_end() {
        let edit = TextEdit::from_text("hello");
        assert_eq!(edit.text(), "hello");
        assert_eq!(edit.caret(), 5);
        assert!(!edit.has_selection());
    }

    #[test]
    fn insert_appends_at_caret() {
        let mut edit = TextEdit::new();
        edit.insert("ab");
        edit.insert("c");
        assert_eq!(edit.text(), "abc");
        assert_eq!(edit.caret(), 3);
    }

    #[test]
    fn insert_replaces_selection() {
        let mut edit = TextEdit::from_text("hello");
        edit.set_caret(1, false);
        edit.set_caret(4, true); // select "ell"
        assert_eq!(edit.selected_text(), "ell");
        edit.insert("ipp");
        assert_eq!(edit.text(), "hippo");
        assert_eq!(edit.caret(), 4);
        assert!(!edit.has_selection());
    }

    #[test]
    fn replace_range_replaces_the_partial_token_and_moves_the_caret() {
        let mut edit = TextEdit::from_text("coun == US");
        // Replace "coun" with "country".
        edit.replace_range(0..4, "country");
        assert_eq!(edit.text(), "country == US");
        assert_eq!(edit.caret(), 7);
        assert!(!edit.has_selection());
        // An empty range inserts at the caret.
        edit.set_caret(0, false);
        edit.replace_range(0..0, "x");
        assert_eq!(edit.text(), "xcountry == US");
        assert_eq!(edit.caret(), 1);
        // A range past the end is clamped rather than panicking.
        edit.replace_range(100..200, "!");
        assert_eq!(edit.text(), "xcountry == US!");
        assert_eq!(edit.caret(), edit.text().len());
    }

    #[test]
    fn backspace_removes_previous_char() {
        let mut edit = TextEdit::from_text("abc");
        assert!(edit.backspace());
        assert_eq!(edit.text(), "ab");
        assert_eq!(edit.caret(), 2);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut edit = TextEdit::new();
        assert!(!edit.backspace());
        assert_eq!(edit.text(), "");
    }

    #[test]
    fn backspace_removes_selection() {
        let mut edit = TextEdit::from_text("abcdef");
        edit.set_caret(1, false);
        edit.set_caret(4, true);
        assert!(edit.backspace());
        assert_eq!(edit.text(), "aef");
        assert_eq!(edit.caret(), 1);
    }

    #[test]
    fn delete_removes_next_char() {
        let mut edit = TextEdit::from_text("abc");
        edit.set_caret(1, false);
        assert!(edit.delete());
        assert_eq!(edit.text(), "ac");
        assert_eq!(edit.caret(), 1);
    }

    #[test]
    fn delete_removes_selection() {
        let mut edit = TextEdit::from_text("abcdef");
        edit.set_caret(1, false);
        edit.set_caret(4, true);
        assert!(edit.delete());
        assert_eq!(edit.text(), "aef");
    }

    #[test]
    fn move_left_from_selection_collapses_to_start() {
        let mut edit = TextEdit::from_text("abcdef");
        edit.set_caret(1, false);
        edit.set_caret(4, true);
        edit.move_left(false);
        assert_eq!(edit.caret(), 1);
        assert!(!edit.has_selection());
    }

    #[test]
    fn move_right_from_selection_collapses_to_end() {
        let mut edit = TextEdit::from_text("abcdef");
        edit.set_caret(1, false);
        edit.set_caret(4, true);
        edit.move_right(false);
        assert_eq!(edit.caret(), 4);
        assert!(!edit.has_selection());
    }

    #[test]
    fn shift_arrow_extends_selection() {
        let mut edit = TextEdit::from_text("abcdef");
        edit.set_caret(2, false);
        edit.move_right(true);
        edit.move_right(true);
        assert_eq!(edit.selection(), 2..4);
        assert_eq!(edit.selected_text(), "cd");
    }

    #[test]
    fn home_and_end_jump_to_edges() {
        let mut edit = TextEdit::from_text("abcdef");
        edit.move_home(false);
        assert_eq!(edit.caret(), 0);
        edit.move_end(false);
        assert_eq!(edit.caret(), 6);
        edit.move_home(true);
        assert_eq!(edit.selection(), 0..6);
    }

    #[test]
    fn select_all_then_move_right_collapses_to_end() {
        let mut edit = TextEdit::from_text("hello");
        edit.select_all();
        assert_eq!(edit.selection(), 0..5);
        edit.move_right(false);
        assert_eq!(edit.caret(), 5);
        assert!(!edit.has_selection());
    }

    #[test]
    fn word_movement_crosses_operator_tokens() {
        let mut edit = TextEdit::from_text("country == US");
        edit.move_home(false);
        edit.move_word_right(false);
        assert_eq!(edit.caret(), 7); // after "country"
        edit.move_word_right(false);
        assert_eq!(edit.caret(), 10); // after "=="
        edit.move_word_left(false);
        assert_eq!(edit.caret(), 8); // before "=="
    }

    #[test]
    fn caret_snaps_to_char_boundary() {
        let mut edit = TextEdit::from_text("aé"); // 'é' is two bytes
                                                  // Byte 2 is in the middle of 'é'; setting there must snap back.
        edit.set_caret(2, false);
        assert!(edit.text().is_char_boundary(edit.caret()));
        assert_eq!(edit.caret(), 1);
    }

    #[test]
    fn backspace_on_multibyte_removes_whole_char() {
        let mut edit = TextEdit::from_text("aé");
        assert!(edit.backspace());
        assert_eq!(edit.text(), "a");
    }

    #[test]
    fn set_text_resets_selection() {
        let mut edit = TextEdit::from_text("hello");
        edit.select_all();
        edit.set_text("new");
        assert_eq!(edit.text(), "new");
        assert_eq!(edit.caret(), 3);
        assert!(!edit.has_selection());
    }
}

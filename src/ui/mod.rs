//! UI module for the Anvil binary.
//!
//! Modules:
//!  - `app`: top-level GPUI application + window lifecycle
//!  - `view`: the main inspection view (toolbar, grid, status bar); its
//!    `search`, `filter`, and `tests` submodules hold the bars and the
//!    UI suite
//!  - `commands`: keyboard chord -> action binding table
//!  - `grid`: virtualized table renderer
//!  - `input`: shared editable-text rendering / hit-testing helpers
//!  - `text_edit`: GPUI-free single-line edit buffer
//!  - `archive_picker`: in-archive file selector helpers
//!  - `theme`: small collection of colours / metrics

pub mod app;
pub mod archive_picker;
mod commands;
pub mod grid;
mod input;
mod text_edit;
pub mod theme;
pub mod view;

/// Human-readable name for a delimiter byte.
pub fn describe_delimiter(d: u8) -> &'static str {
    match d {
        b',' => "comma",
        b'\t' => "tab",
        b';' => "semicolon",
        b'|' => "pipe",
        _ => "custom",
    }
}

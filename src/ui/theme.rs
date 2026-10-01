//! Small set of colours and metrics used across the UI.
//!
//! Designed to be information-dense and easy on the eyes for extended
//! sessions. Dark is the default; we keep the palette in one place so a
//! light mode can be added later without touching every view.

use gpui::{px, Pixels, Rgba};

/// `gpui::rgb` is not `const`, so we reproduce it here to keep the palette
/// as compile-time constants.
const fn rgb(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

pub const BG: Rgba = rgb(0x1e1e1e);
pub const BG_PANEL: Rgba = rgb(0x252526);
pub const BG_TOOLBAR: Rgba = rgb(0x2d2d30);
pub const BG_STATUS: Rgba = rgb(0x2d2d30);
pub const BG_HEADER: Rgba = rgb(0x333337);
pub const BG_ROW_ALT: Rgba = rgb(0x232326);
pub const BG_ROW_HOVER: Rgba = rgb(0x2a2d2e);
pub const BG_SELECTED: Rgba = rgb(0x094771);
/// Brief green flash painted over the selection right after a copy, as
/// positive confirmation that the clipboard write happened.
pub const BG_COPIED: Rgba = rgb(0x2f8f4f);
/// Background tint for rows that match the current search query. Every
/// matched row gets this colour so the user can scan a long result
/// list at a glance.
pub const BG_MATCH: Rgba = rgb(0x4a3a00);
/// Slightly stronger tint for the row currently selected by the
/// search (the "active" match).
pub const BG_MATCH_ACTIVE: Rgba = rgb(0x6b5400);
/// Bright highlight drawn behind the literal matched substring inside
/// each cell. Distinct from the row tint so the user can see not just
/// "this row matches" but also "this part of the cell is the match".
pub const BG_MATCH_TEXT: Rgba = rgb(0xc69400);

pub const BORDER: Rgba = rgb(0x3c3c3c);
pub const BORDER_STRONG: Rgba = rgb(0x505050);

pub const TEXT: Rgba = rgb(0xd4d4d4);
pub const TEXT_DIM: Rgba = rgb(0x9d9d9d);
pub const TEXT_STRONG: Rgba = rgb(0xffffff);
pub const TEXT_EMPTY: Rgba = rgb(0x666666);

pub const ACCENT: Rgba = rgb(0x0e639c);
pub const WARN: Rgba = rgb(0xd7ba7d);
/// Error emphasis (e.g. an unterminated-quote row marker).
pub const ERROR: Rgba = rgb(0xe06c75);

pub const TOOLBAR_HEIGHT: Pixels = px(38.);
pub const TAB_BAR_HEIGHT: Pixels = px(28.);
pub const STATUS_HEIGHT: Pixels = px(24.);
/// Width of the left-hand file-manager panel.
pub const FILE_PANEL_WIDTH: Pixels = px(220.);
/// Width of the right-hand column-inspector panel.
pub const INSPECTOR_WIDTH: Pixels = px(320.);
/// Width of the right-hand diagnostics panel.
pub const DIAGNOSTICS_WIDTH: Pixels = px(320.);
/// Left padding for a file row nested under an archive group header.
pub const FILE_PANEL_ENTRY_INDENT: Pixels = px(18.);
pub const HEADER_HEIGHT: Pixels = px(24.);
pub const ROW_HEIGHT: Pixels = px(22.);
pub const ROW_NUMBER_WIDTH: Pixels = px(64.);
pub const CELL_PAD_X: Pixels = px(6.);

/// Default column width when we have no sampled width.
pub const DEFAULT_COLUMN_WIDTH: Pixels = px(140.);
pub const MIN_COLUMN_WIDTH: Pixels = px(48.);
/// Upper bound for a manually resized column. Wider than the automatic
/// `MAX_AUTO_WIDTH` so a user can deliberately stretch a column.
pub const MAX_COLUMN_WIDTH: Pixels = px(1200.);
/// Maximum number of columns whose width we actually measure. Beyond this
/// we use the default and let virtualization hide the cost.
pub const MAX_MEASURED_COLUMNS: usize = 4096;

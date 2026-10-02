//! Semantic color snapshots and physical grid metrics.
//!
//! Colors follow the active Kit theme. Grid geometry deliberately remains in
//! physical pixels: row virtualization, text measurement and hit testing share
//! these metrics, independently of the application's control density.
//!
//! The palette is organized by *surface role* rather than by hue. The window
//! is built as a stack of translucent layers over one opaque canvas, so a
//! renderer's job is only to pick the layer that matches its job:
//!
//! ```text
//!   canvas ── the opaque backdrop; nothing is drawn straight onto the window
//!     └─ wash      a soft light gradient near the top (chrome only)
//!         └─ chrome     translucent; the wash reads through it
//!     └─ strip     recessed; documents sit *below* the chrome
//!     └─ panel     translucent side surfaces
//!     └─ surface   fully opaque; the grid is the working surface
//! ```
//!
//! Every value is derived from the active Kit theme so Anvil Slate (dark) and
//! Anvil Warm (light) stay in step automatically.

use gpui_kit::component::{ActiveTheme, Colorize, Theme, ThemeConfig, ThemeMode};
use gpui_kit::{px, App, Hsla, Pixels};

/// A frame-local, allocation-free projection of Kit's semantic colors.
/// Pass this value into render helpers that do not otherwise need an App.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Palette {
    // ---- structure ------------------------------------------------------
    /// Opaque window backdrop. Nothing is painted directly onto the window.
    pub canvas: Hsla,
    /// Translucent command chrome (command bar, floating surfaces).
    pub chrome: Hsla,
    /// Recessed document strip that carries the tabs.
    pub strip: Hsla,
    /// Translucent side panels.
    pub panel: Hsla,
    /// Opaque data surface: grid rows and column headers.
    pub surface: Hsla,
    /// The row-number gutter, a shade darker than the data surface.
    pub gutter: Hsla,
    /// Column header band.
    pub header: Hsla,
    /// Opaque exact-value material shared by auxiliary panels.
    pub data_well: Hsla,

    // ---- data -----------------------------------------------------------
    pub row_alternate: Hsla,
    pub hover: Hsla,
    pub selection: Hsla,
    pub selection_border: Hsla,
    pub copied: Hsla,
    pub search_match: Hsla,
    pub search_active: Hsla,
    pub search_text: Hsla,
    pub focus_ring: Hsla,

    // ---- lines and text -------------------------------------------------
    pub border: Hsla,
    pub border_strong: Hsla,
    pub row_border: Hsla,
    pub gutter_rule: Hsla,
    pub foreground: Hsla,
    pub muted_foreground: Hsla,
    pub faint_foreground: Hsla,
    pub strong_foreground: Hsla,
    pub empty_foreground: Hsla,
    pub primary: Hsla,
    /// Pressed / selected wash for chrome controls.
    pub chrome_active: Hsla,

    // ---- elevation ------------------------------------------------------
    /// Soft shadow under a surface that floats above the grid (query bars).
    /// Ink-coloured rather than black so it reads as occlusion, not as dirt.
    pub shadow: Hsla,

    pub warning: Hsla,
    pub danger: Hsla,
}

impl Palette {
    pub fn from_theme(theme: &Theme) -> Self {
        let colors = theme.color_tokens();
        let canvas = colors.background;
        let dark = theme.is_dark();
        // Row-level fills are painted *directly onto* the data surface rather
        // than composited over whatever happens to be underneath, so they are
        // blended against `surface` explicitly. That keeps a search match,
        // a selection and a copy flash identical whether they land on an even
        // row, an odd row or the gutter.
        let surface = blend(canvas, colors.foreground, 0.03).alpha(1.);
        Self {
            canvas,
            chrome: blend(canvas, lift(canvas, colors.primary, 0.06), 0.58).alpha(if dark {
                0.58
            } else {
                0.72
            }),
            strip: colors.muted.alpha(0.5),
            panel: blend(canvas, theme.sidebar, 0.75).alpha(0.94),
            surface,
            gutter: gutter(canvas, colors.foreground, dark),
            header: blend(canvas, colors.muted, 0.85).alpha(1.),
            data_well: blend(blend(canvas, theme.sidebar, 0.75), canvas, 0.6).alpha(1.),

            row_alternate: blend(surface, colors.foreground, 0.022),
            hover: blend(surface, colors.foreground, 0.055),
            selection: blend(
                surface,
                colors.selection.alpha(1.),
                if dark { 0.26 } else { 0.20 },
            ),
            selection_border: colors.selection,
            copied: blend(surface, theme.success, if dark { 0.30 } else { 0.24 }),
            // Search is an informational state, not a data-quality warning.
            search_match: blend(surface, theme.info, if dark { 0.12 } else { 0.14 }),
            search_active: blend(surface, theme.info, if dark { 0.28 } else { 0.26 }),
            search_text: blend(surface, theme.info, if dark { 0.48 } else { 0.34 }),
            focus_ring: theme.ring.alpha(0.5),

            border: colors.border,
            border_strong: blend(colors.border, colors.foreground, 0.22),
            row_border: blend(canvas, colors.border, 0.5),
            gutter_rule: blend(canvas, colors.border, 0.35),
            foreground: colors.foreground,
            muted_foreground: colors.muted_foreground,
            faint_foreground: colors.muted_foreground.alpha(if dark { 0.7 } else { 0.75 }),
            strong_foreground: colors.foreground,
            empty_foreground: colors.muted_foreground.alpha(0.65),
            primary: colors.primary,
            chrome_active: colors.accent,

            shadow: gpui_kit::black().alpha(if dark { 0.34 } else { 0.10 }),

            warning: theme.warning,
            danger: theme.danger,
        }
    }
}

/// `amount` of `color` layered over `surface`.
///
/// Kit's `mix_oklab` weights its **receiver**: `a.mix_oklab(b, f)` is
/// `f * a + (1 - f) * b`. Routing every blend through this helper keeps that
/// inversion in one place instead of at every call site, where a single
/// transposed pair silently renders a dark surface as near-white.
pub fn blend(surface: Hsla, color: Hsla, amount: f32) -> Hsla {
    color.mix_oklab(surface, amount.clamp(0., 1.))
}

/// `amount` of `color` layered over `surface`, with `color`'s hue shifted a
/// little toward the surface first. Used for chrome, where a trace of the
/// accent hue is what makes the top of the window read as lit.
fn lift(surface: Hsla, color: Hsla, amount: f32) -> Hsla {
    blend(surface, color, amount)
}

/// The opaque data surface: `canvas` lifted a hair toward the text color so
/// the grid reads as a sheet laid on the window rather than a hole in it.
#[cfg(test)]
fn surface(canvas: Hsla, foreground: Hsla, amount: f32) -> Hsla {
    blend(canvas, foreground, amount)
}

/// The row-number gutter, a shade *below* the data surface. Dark mode has
/// nowhere darker to go than the canvas itself, so it steps toward black
/// instead of toward the text color.
fn gutter(canvas: Hsla, foreground: Hsla, dark: bool) -> Hsla {
    if dark {
        blend(gpui_kit::black(), canvas, 0.86)
    } else {
        blend(canvas, foreground, 0.045)
    }
}

/// Read the current theme once per rendered region (or virtual-list batch).
pub fn palette(cx: &App) -> Palette {
    Palette::from_theme(cx.theme())
}

/// Install both modes after `gpui_kit::init`. The parent toggle can call
/// `Theme::change(mode, Some(window), cx)` directly: registered palettes survive
/// mode switches and Kit refreshes all windows and its Base projection.
pub fn init(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.light_theme = std::rc::Rc::new(config((*theme.light_theme).clone(), false));
        theme.dark_theme = std::rc::Rc::new(config((*theme.dark_theme).clone(), true));
    });
    Theme::change(ThemeMode::Dark, None, cx);
}

fn config(mut theme: ThemeConfig, dark: bool) -> ThemeConfig {
    theme.name = if dark { "Anvil Slate" } else { "Anvil Warm" }.into();
    theme.mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    let colors = &mut theme.colors;
    macro_rules! color {
        ($field:ident, $dark:literal, $light:literal) => {
            colors.$field = Some(if dark { $dark } else { $light }.into());
        };
    }
    // Surfaces, from the window backdrop outward.
    color!(background, "#0D131C", "#EFEDE7");
    color!(sidebar, "#141C28", "#F7F5F1");
    color!(group_box, "#1B2634", "#FFFFFF");
    color!(popover, "#1A2432", "#FFFDFA");
    color!(muted, "#1D2836", "#E3DFD7");
    // Text.
    color!(foreground, "#DDE5EF", "#2A2C31");
    color!(muted_foreground, "#8797AB", "#62656C");
    color!(sidebar_foreground, "#DDE5EF", "#2A2C31");
    color!(group_box_foreground, "#DDE5EF", "#2A2C31");
    color!(popover_foreground, "#DDE5EF", "#2A2C31");
    // Structure.
    color!(accent, "#212D3D", "#E6E2DA");
    color!(accent_foreground, "#DDE5EF", "#2A2C31");
    color!(border, "#222E3D", "#DCD8D0");
    color!(input, "#33445A", "#C6C2B9");
    color!(title_bar, "#1B2634", "#FFFFFF");
    color!(title_bar_border, "#222E3D", "#DCD8D0");
    color!(status_bar, "#141C28", "#F1EEE8");
    color!(status_bar_border, "#222E3D", "#DCD8D0");
    color!(overlay, "#0A0F1699", "#23201966");
    // Accents.
    color!(primary, "#C58B68", "#8A4E2F");
    color!(primary_hover, "#D59B78", "#754025");
    color!(primary_active, "#AC7353", "#63351F");
    color!(primary_foreground, "#17120F", "#FFFFFF");
    // Set component overrides too: an inherited config may already define them.
    colors.button_primary = colors.primary.clone();
    colors.button_primary_hover = colors.primary_hover.clone();
    colors.button_primary_active = colors.primary_active.clone();
    colors.button_primary_foreground = colors.primary_foreground.clone();
    color!(selection, "#5B9BF0", "#2A63C8");
    color!(ring, "#C58B68", "#8A4E2F");
    color!(info, "#74ACF2", "#2A63C8");
    color!(success, "#6FBB95", "#2A7348");
    color!(warning, "#D9B56F", "#8A6218");
    color!(danger, "#E89098", "#B23B47");
    theme
}

/// Shared inspector geometry and type scale (physical layout pixels).
pub const PANEL_INSET: f32 = 12.;
pub const PANEL_HEADER_HEIGHT: Pixels = px(30.);
pub const PANEL_SUBJECT_SIZE: Pixels = px(14.);
pub const PANEL_SUBJECT_LINE_HEIGHT: Pixels = px(20.);
pub const PANEL_SECTION_SIZE: Pixels = px(11.);
pub const PANEL_SECTION_LINE_HEIGHT: Pixels = px(16.);

pub const STATUS_HEIGHT: Pixels = px(22.);

/// Height of the persistent command bar.
pub const COMMAND_BAR_HEIGHT: Pixels = px(36.);
/// Height of the document strip that carries the tabs.
pub const DOCUMENT_STRIP_HEIGHT: Pixels = px(32.);

/// Width of the left-hand file-manager panel.
pub const FILE_PANEL_WIDTH: Pixels = px(224.);
/// Width of the right-hand column-inspector panel.
pub const INSPECTOR_WIDTH: Pixels = px(316.);
/// Width of the right-hand diagnostics panel.
pub const DIAGNOSTICS_WIDTH: Pixels = px(316.);
/// Width of the right-hand full-cell detail panel.
pub const CELL_DETAIL_WIDTH: Pixels = px(360.);

pub const HEADER_HEIGHT: Pixels = px(26.);
pub const ROW_HEIGHT: Pixels = px(22.);
pub const ROW_NUMBER_WIDTH: Pixels = px(58.);
pub const CELL_PAD_X: Pixels = px(8.);

/// Default column width when we have no sampled width.
pub const DEFAULT_COLUMN_WIDTH: Pixels = px(140.);
pub const MIN_COLUMN_WIDTH: Pixels = px(48.);
/// Upper bound for a manually resized column. Wider than the automatic
/// `MAX_AUTO_WIDTH` so a user can deliberately stretch a column.
pub const MAX_COLUMN_WIDTH: Pixels = px(1200.);
/// Maximum number of columns whose width we actually measure. Beyond this
/// we use the default and let virtualization hide the cost.
pub const MAX_MEASURED_COLUMNS: usize = 4096;

/// WCAG relative luminance: linearize sRGB before applying channel weights.
#[cfg(test)]
fn luminance(color: Hsla) -> f32 {
    let rgb = color.to_rgb();
    fn linear(channel: f32) -> f32 {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn palette_follows_mode_changes_and_custom_tokens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let dark = palette(cx);
            assert!(
                (dark.search_match.to_rgb().r - dark.canvas.to_rgb().r).abs() < 0.2,
                "search rows should be a tint of the surface, not an accent fill: {:?}",
                dark.search_match
            );
            assert_eq!(
                dark.search_match.a, 1.0,
                "a search match is an opaque row fill, not a translucent overlay"
            );
            assert_eq!(cx.theme().theme_name().as_ref(), "Anvil Slate");
            Theme::change(ThemeMode::Light, None, cx);
            let light = palette(cx);
            assert_eq!(cx.theme().theme_name().as_ref(), "Anvil Warm");
            assert_ne!(dark.canvas, light.canvas);
            assert_ne!(dark.foreground, light.foreground);
            assert_ne!(dark.selection, light.selection);
            assert_ne!(light.search_match, light.search_active);
            assert_ne!(light.search_active, light.search_text);
            assert_ne!(light.copied, light.selection);
            Theme::update(cx, |theme| theme.warning = light.danger);
            assert_eq!(palette(cx).warning, light.danger);
            Theme::change(ThemeMode::Dark, None, cx);
            assert_eq!(palette(cx).canvas, dark.canvas);
            assert_eq!(palette(cx).warning, dark.warning);
        });
    }

    #[gpui_kit::test]
    fn chrome_is_translucent_over_an_opaque_canvas_in_both_appearances(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            for mode in [ThemeMode::Dark, ThemeMode::Light] {
                Theme::change(mode, None, cx);
                let p = palette(cx);
                assert!(
                    p.chrome.a < 1.0,
                    "chrome must let the window backdrop read through it ({mode:?})"
                );
                assert_eq!(
                    p.surface.a, 1.0,
                    "the grid must stay opaque so values remain readable ({mode:?})"
                );
                assert!(
                    luminance(p.surface) > luminance(p.gutter),
                    "the gutter must sit below the data surface ({mode:?})"
                );
            }
        });
    }

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[gpui_kit::test]
    fn copper_buttons_and_opaque_data_materials_meet_contrast(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            for mode in [ThemeMode::Dark, ThemeMode::Light] {
                Theme::change(mode, None, cx);
                let theme = cx.theme();
                let p = palette(cx);
                for (state, fill) in [
                    ("default", theme.button_primary),
                    ("hover", theme.button_primary_hover),
                    ("pressed", theme.button_primary_active),
                ] {
                    let ratio = contrast(theme.button_primary_foreground, fill);
                    assert!(ratio >= 4.5, "{mode:?} primary {state}: {ratio:.3}:1");
                    assert_eq!(fill.a, 1.);
                }
                assert_eq!(theme.button_primary, theme.primary);
                assert_eq!(theme.button_primary_hover, theme.primary_hover);
                assert_eq!(theme.button_primary_active, theme.primary_active);
                assert_eq!(theme.button_primary_foreground, theme.primary_foreground);
                assert_eq!(theme.ring, theme.primary);
                assert_ne!(theme.primary, theme.info);
                assert_ne!(theme.primary, theme.selection);
                assert_eq!(p.selection_border, theme.selection);
                for (name, fill) in [
                    ("surface", p.surface),
                    ("well", p.data_well),
                    ("gutter", p.gutter),
                    ("header", p.header),
                    ("alternate", p.row_alternate),
                    ("hover", p.hover),
                    ("selection", p.selection),
                    ("copied", p.copied),
                    ("search", p.search_match),
                    ("active search", p.search_active),
                    ("search text", p.search_text),
                ] {
                    assert_eq!(fill.a, 1., "{mode:?} {name} must be opaque");
                }
                assert!(
                    theme.overlay.a > 0. && theme.overlay.a < 1.,
                    "modal scrims must preserve workspace context"
                );
                for (name, fill) in [
                    ("surface", p.surface),
                    ("well", p.data_well),
                    ("gutter", p.gutter),
                ] {
                    for (text_name, text) in
                        [("foreground", p.foreground), ("muted", p.muted_foreground)]
                    {
                        let ratio = contrast(text, fill);
                        assert!(ratio >= 4.5, "{mode:?} {text_name} on {name}: {ratio:.3}:1");
                    }
                }
            }
        });
    }

    #[test]
    fn wcag_luminance_reference_values() {
        assert_eq!(luminance(gpui_kit::black()), 0.);
        assert!((luminance(gpui_kit::white()) - 1.).abs() < 0.00001);
        assert!((contrast(gpui_kit::black(), gpui_kit::white()) - 21.).abs() < 0.00001);
        assert!((luminance(gpui_kit::rgb(0x808080).into()) - 0.21586).abs() < 0.0001);
    }

    #[test]
    fn blend_puts_the_requested_amount_of_color_on_top() {
        let dark = gpui_kit::black().alpha(1.);
        let light = gpui_kit::white().alpha(1.);
        // Kit's `mix_oklab` weights its receiver; `blend` must invert that so
        // a small `amount` stays close to the surface.
        let result = blend(dark, light, 0.1);
        assert!(
            luminance(result) < 0.35,
            "10% of white over black must stay dark, got {result:?}"
        );
        assert_eq!(result.a, 1.0, "blending two opaque colours stays opaque");
    }

    #[test]
    fn gutter_sits_below_the_data_surface_in_both_appearances() {
        let canvas = gpui_kit::Hsla {
            h: 0.,
            s: 0.,
            l: 0.5,
            a: 1.,
        };
        let surface = surface(canvas, gpui_kit::white(), 0.03);
        assert!(luminance(surface) > luminance(gutter(canvas, gpui_kit::white(), true)));
        assert!(luminance(surface) > luminance(gutter(canvas, gpui_kit::black(), false)));
    }

    #[test]
    fn relative_luminance_orders_the_dark_layers() {
        let canvas = gpui_kit::Hsla {
            h: 214.,
            s: 0.32,
            l: 0.10,
            a: 1.,
        };
        assert!(luminance(blend(canvas, gpui_kit::white(), 0.022)) > luminance(canvas));
        assert!(
            luminance(blend(canvas, gpui_kit::white(), 0.05))
                > luminance(blend(canvas, gpui_kit::white(), 0.022))
        );
    }
}

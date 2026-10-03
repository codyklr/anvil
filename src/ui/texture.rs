//! Procedural surface texture: dithered fields, soft washes and edge fades.
//!
//! GPUI 0.3 exposes no backdrop-filter or image-wash background, so the
//! material language is built from primitives that are cheap enough to
//! repaint on every frame:
//!
//! * [`wash`] - a directional gradient painted as a real GPU background,
//!   used to light the top of the window and to fade a region's edge.
//! * [`stipple`] - a deterministic dot field whose density follows a radial
//!   falloff. This is the halftone/dither treatment: it gives the empty state
//!   an atmospheric centre without a gradient edge, and never sits under
//!   dense data. The halo is capped at [`MAX_REACH`], so the field's cost
//!   is constant in the size of the region it sits in.
//! * [`grain`] - a fine noise field for large quiet surfaces, painted as a
//!   GPU checkerboard pattern.
//! * [`chrome_dither`] - the same halftone language for the main menu
//!   chrome, painted as a GPU slash pattern.
//!
//! * [`home_field`] - a cached, ordered Bayer screen depicting a blocky
//!   isometric anvil, independent of the legacy radial stipple reference.
//!
//! [`grain`] and [`chrome_dither`] are `Background` patterns evaluated
//! per-fragment in the GPU shader: they avoid CPU dot enumeration, but
//! still incur GPU fill cost. [`stipple`] generates a bounded dot list
//! per paint; [`home_field`] reuses fixed geometry and submits bounded
//! quads without regenerating or allocating a dot list.

use std::sync::OnceLock;

use gpui_kit::{
    canvas, checkerboard, div, linear_gradient, pattern_slash, prelude::*, px, App, Background,
    BorderStyle, Bounds, Corners, Div, Edges, Hsla, LinearColorStop, PaintQuad, Pixels, Point,
    Size, Window,
};

/// Deterministic 2D hash in `[0, 1)`.
///
/// A cheap integer avalanche is enough: the field only needs to look
/// uncorrelated, and being stable across frames matters more than being
/// statistically perfect.
#[cfg(test)]
fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_f491);
    h ^= h >> 13;
    h ^= seed.wrapping_mul(0x9e37_79b9);
    let h = (h >> 8) & 0x00ff_ffff;
    h as f32 / 0x0100_0000 as f32
}

/// How far from its focus point a [`stipple`] halo reaches, in pixels.
///
/// The cap is what keeps the field cheap: a huge window gets the same
/// centred halo as a small one, never a full-window pixel loop.
#[cfg(test)]
const MAX_REACH: f32 = 260.;

/// How a [`stipple`] field is shaped.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) enum StippleShape {
    /// Density falls off from a point at `focus` across the region's width.
    /// `spread` controls how quickly it thins.
    Radial { focus: f32, spread: f32 },
}

/// The dot offsets of a stipple field over a `width` x `height` region.
///
/// Pure and deterministic: the same region and parameters always yield
/// the same offsets, expressed relative to the region's origin so the
/// result does not depend on where the region sits in the window.
///
/// Only the lattice cells inside the halo's bounding box are walked,
/// and the halo itself is capped at [`MAX_REACH`], so the work depends
/// on the field's own geometry rather than on the region's size.
#[cfg(test)]
fn stipple_dots(
    width: f32,
    height: f32,
    shape: StippleShape,
    step: f32,
    seed: u32,
) -> Vec<(f32, f32)> {
    let StippleShape::Radial { focus, spread } = shape;
    let focus_x = width * focus;
    let focus_y = height * 0.5;
    let reach = (width.min(height) * spread).min(MAX_REACH);
    let mut dots = Vec::new();
    if reach <= 0. {
        return dots;
    }
    let cols = (width / step) as i32;
    let rows = (height / step) as i32;
    // Walk only the cells inside the halo's bounding box, keeping the
    // lattice indices global so the field stays put when the region
    // around it grows.
    let col_lo = ((focus_x - reach) / step).max(0.).floor() as i32;
    let col_hi = ((focus_x + reach) / step).ceil() as i32;
    let row_lo = ((focus_y - reach) / step).max(0.).floor() as i32;
    let row_hi = ((focus_y + reach) / step).ceil() as i32;
    for row in row_lo..row_hi.min(rows) {
        for col in col_lo..col_hi.min(cols) {
            // Sub-cell jitter breaks the lattice so the field reads as
            // stipple rather than as a dot screen.
            let x = col as f32 * step + hash2(col, row, seed) * step;
            let y = row as f32 * step + hash2(col, row, seed ^ 0x9e37_79b9) * step;
            let dx = x - focus_x;
            let dy = y - focus_y;
            let distance = (dx * dx + dy * dy).sqrt();
            let density = (1. - distance / reach).clamp(0., 1.);
            // Threshold the local density against an ordered roll: the
            // field dissolves gradually instead of ending on a hard
            // disc edge.
            if density <= 0.02 || hash2(col, row, seed ^ 0x5bf0_3635) > density {
                continue;
            }
            dots.push((x, y));
        }
    }
    dots
}

/// A dithered / halftone dot field.
///
/// `step` is the grid pitch in pixels; `strength` is the alpha of a fully
/// dense dot. Both are chosen by the caller so the effect stays subtle: the
/// point is to break up a flat field, never to look like a screen door.
///
/// The field is painted on a `canvas`, so it runs on the UI thread once
/// per render pass - which is why the halo is bounded (see
/// [`stipple_dots`]) instead of covering the whole region.
// Retained as a test-only reference for the earlier radial material.
#[cfg(test)]
#[allow(dead_code)]
pub(super) fn stipple(
    shape: StippleShape,
    step: f32,
    strength: f32,
    color: Hsla,
    seed: u32,
) -> impl IntoElement {
    canvas(
        move |_bounds, _window, _cx| shape,
        move |bounds, shape, window: &mut Window, _cx: &mut App| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            if width < step * 2. || height < step * 2. {
                return;
            }
            let dots = stipple_dots(width, height, shape, step, seed);
            let tint = color.alpha(color.a * strength);
            let size = step * 0.42;
            for (x, y) in dots {
                window.paint_quad(PaintQuad {
                    bounds: Bounds::new(
                        Point::new(
                            px(f32::from(bounds.origin.x) + x),
                            px(f32::from(bounds.origin.y) + y),
                        ),
                        Size::new(px(size), px(size)),
                    ),
                    corner_radii: Corners::default(),
                    background: tint.into(),
                    border_widths: Edges::all(px(0.)),
                    border_color: Hsla::transparent_black(),
                    border_style: BorderStyle::Solid,
                });
            }
        },
    )
    .absolute()
    .inset(px(0.))
}

const HOME_WIDTH: f32 = 520.;
const HOME_HEIGHT: f32 = 420.;
const HOME_PITCH: f32 = 6.;
const HOME_DOT_SIZE: f32 = 2.5;
const BAYER_4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
static HOME_HEAD: OnceLock<Vec<(f32, f32)>> = OnceLock::new();
static HOME_BODY: OnceLock<Vec<(f32, f32)>> = OnceLock::new();

/// Face tones of the striking block (top, front, right). The waist and
/// base use every other tone in [`ANVIL_FACES`].
fn is_head_tone(tone: f32) -> bool {
    tone == 0.92 || tone == 0.66 || tone == 0.44
}

/// Convex faces, back to front: plinth, narrow waist, broad striking block.
/// The first five faces are the waist and base; the last three are the lit
/// head. Separate screen densities give the isometric form depth without
/// images, gradients or lighting tasks. The head and the body are cached
/// apart so each mass can take its own ink.
const ANVIL_FACES: [([(f32, f32); 4], f32); 8] = [
    (
        [(102., 300.), (294., 222.), (418., 284.), (226., 362.)],
        0.78,
    ),
    (
        [(102., 300.), (226., 362.), (226., 398.), (102., 336.)],
        0.62,
    ),
    (
        [(226., 362.), (418., 284.), (418., 320.), (226., 398.)],
        0.40,
    ),
    (
        [(188., 242.), (246., 272.), (246., 324.), (188., 294.)],
        0.58,
    ),
    (
        [(246., 272.), (332., 238.), (332., 290.), (246., 324.)],
        0.32,
    ),
    ([(90., 128.), (294., 46.), (430., 114.), (226., 196.)], 0.92),
    ([(90., 128.), (226., 196.), (226., 256.), (90., 188.)], 0.66),
    (
        [(226., 196.), (430., 114.), (430., 174.), (226., 256.)],
        0.44,
    ),
];

fn anvil_tone(x: f32, y: f32) -> f32 {
    ANVIL_FACES
        .iter()
        .rev()
        .find(|(face, _)| {
            // All faces share the same winding. Shared edges belong to the
            // foreground face, preventing seams in the ordered screen.
            (0..4).all(|i| {
                let (ax, ay) = face[i];
                let (bx, by) = face[(i + 1) % 4];
                (bx - ax) * (y - ay) - (by - ay) * (x - ax) >= 0.
            })
        })
        .map_or(0., |(_, tone)| *tone)
}

/// Ordered-dither dots for one mass. `head` selects the striking block;
/// otherwise the waist and base. Tone is occupancy, not alpha.
fn home_mass(head: bool) -> Vec<(f32, f32)> {
    let mut dots = Vec::new();
    for row in 0..(HOME_HEIGHT / HOME_PITCH) as usize {
        for col in 0..(HOME_WIDTH / HOME_PITCH) as usize {
            let x = col as f32 * HOME_PITCH;
            let y = row as f32 * HOME_PITCH;
            let tone = anvil_tone(x + HOME_DOT_SIZE * 0.5, y + HOME_DOT_SIZE * 0.5);
            if is_head_tone(tone) != head {
                continue;
            }
            let threshold = (BAYER_4[row % 4][col % 4] as f32 + 0.5) / 16.;
            if tone > threshold {
                dots.push((x, y));
            }
        }
    }
    dots
}

fn home_head() -> Vec<(f32, f32)> {
    home_mass(true)
}

fn home_body() -> Vec<(f32, f32)> {
    home_mass(false)
}

/// Both masses in lattice order, for geometry checks. Not a third cache.
#[cfg(test)]
fn home_dots() -> Vec<(f32, f32)> {
    let mut dots = home_body();
    dots.extend(home_head());
    dots
}

fn paint_dots(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    dots: &[(f32, f32)],
    color: Hsla,
    left: f32,
    top: f32,
    scale: f32,
    width: f32,
    height: f32,
) {
    let size = HOME_DOT_SIZE * scale;
    for &(x, y) in dots {
        let x = left + x * scale;
        let y = top + y * scale;
        let right = (x + size).min(width);
        let bottom = (y + size).min(height);
        let x = x.max(0.);
        let y = y.max(0.);
        if right <= x || bottom <= y {
            continue;
        }
        window.paint_quad(PaintQuad {
            bounds: Bounds::new(
                Point::new(bounds.origin.x + px(x), bounds.origin.y + px(y)),
                Size::new(px(right - x), px(bottom - y)),
            ),
            corner_radii: Corners::default(),
            background: color.into(),
            border_widths: Edges::all(px(0.)),
            border_color: Hsla::transparent_black(),
            border_style: BorderStyle::Solid,
        });
    }
}

/// Static ordered-dither anvil art, centred at (68%, 50%) of its
/// parent. The head is `head` (copper) and the waist and base are `body`
/// (muted). The 520 × 420 field only scales down; edge dots are cropped to
/// the canvas. Both masses are cached once and submitted from this single
/// canvas. No timers, no regen, no full-window enumeration.
pub(super) fn home_field(head: Hsla, body: Hsla) -> impl IntoElement {
    let head_dots = HOME_HEAD.get_or_init(home_head);
    let body_dots = HOME_BODY.get_or_init(home_body);
    canvas(
        move |_bounds, _window, _cx| (),
        move |bounds, (), window: &mut Window, _cx: &mut App| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let scale = (width / HOME_WIDTH).min(height / HOME_HEIGHT).min(1.);
            if scale <= 0. {
                return;
            }
            let left = width * 0.68 - HOME_WIDTH * scale * 0.5;
            let top = height * 0.5 - HOME_HEIGHT * scale * 0.5;
            paint_dots(
                window, bounds, body_dots, body, left, top, scale, width, height,
            );
            paint_dots(
                window, bounds, head_dots, head, left, top, scale, width, height,
            );
        },
    )
    .absolute()
    .inset(px(0.))
}

/// A fine noise field for large quiet surfaces.
///
/// This is a GPU checkerboard pattern: at a small pitch and low alpha the
/// alternating cells read as film grain, and because the pattern is
/// evaluated in the shader it avoids CPU dot enumeration, while still
/// incurring GPU fill cost. `size` is the cell pitch in pixels and `alpha` the
/// tint strength; the field carries no seed because the pattern is fixed.
pub(crate) fn grain(size: f32, alpha: f32, color: Hsla) -> Div {
    div()
        .absolute()
        .inset(px(0.))
        .bg(checkerboard(color.alpha(alpha), size))
}

/// Stripe width and gap, in pixels, for the main menu chrome dither.
const DITHER_WIDTH: f32 = 0.7;
const DITHER_INTERVAL: f32 = 2.3;

/// A diagonal micro-dither for the main menu chrome (title bar and
/// command bar).
///
/// `pattern_slash` is evaluated in the GPU shader, so the field repaints
/// without CPU dot enumeration, but still incurs GPU fill cost. Its stripes peak at
/// roughly three quarters of `color`'s alpha, so callers pass a low
/// alpha: `foreground` at 0.10 reads as a faint halftone over the
/// chrome without competing with the controls it sits behind.
pub(super) fn chrome_dither(color: Hsla) -> Div {
    div()
        .absolute()
        .inset(px(0.))
        .bg(pattern_slash(color, DITHER_WIDTH, DITHER_INTERVAL))
}

/// A two-stop directional gradient wash, painted as a real GPU background.
///
/// `angle` is in degrees (0 = top→bottom, increasing clockwise) and `stops`
/// is `((from, color), (to, colour))` with positions in `[0, 1]`. This is the
/// workhorse for "light coming from somewhere": a two-stop wash at low alpha
/// reads as atmosphere rather than as a coloured rectangle.
pub(super) fn wash(angle: f32, stops: ((f32, Hsla), (f32, Hsla))) -> Background {
    let stop = |(at, color): (f32, Hsla)| LinearColorStop {
        color,
        percentage: at.clamp(0., 1.),
    };
    linear_gradient(angle, stop(stops.0), stop(stops.1))
}

/// A soft edge fade: the surface colour at one end grading to transparent.
///
/// Used at the right edge of the grid so a column cut off by the viewport
/// reads as "there is more" rather than as clipped text.
pub(super) fn edge_fade(surface: Hsla, to_right: bool) -> Background {
    let clear = surface.alpha(0.);
    if to_right {
        wash(0., ((0., clear), (1., surface)))
    } else {
        wash(0., ((0., surface), (1., clear)))
    }
}

/// A container that fills its parent and paints `background`.
///
/// GPUI `div`s carry no hitbox unless an interaction handler is attached, so
/// this scrim never intercepts a click; the helper exists so call sites read
/// as layout rather than as absolute-positioning plumbing.
pub(super) fn scrim(background: Background) -> Div {
    div().absolute().inset(px(0.)).bg(background)
}

/// The canvas signature above names `Bounds`/`Pixels` through inference; this
/// keeps the import honest without adding noise at the call sites.
#[allow(dead_code)]
fn _type_anchor(_: Option<Bounds<Pixels>>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_geometry_is_deterministic_and_cached() {
        let head = home_head();
        let body = home_body();
        assert_eq!(head, home_head());
        assert_eq!(body, home_body());
        assert!(!head.is_empty() && !body.is_empty());
        let cached_head = HOME_HEAD.get_or_init(home_head);
        let cached_body = HOME_BODY.get_or_init(home_body);
        assert_eq!(&head, cached_head);
        assert_eq!(&body, cached_body);
        assert!(std::ptr::eq(cached_head, HOME_HEAD.get_or_init(home_head)));
        assert!(std::ptr::eq(cached_body, HOME_BODY.get_or_init(home_body)));
        // The two caches partition the silhouette; nothing is drawn twice.
        for dot in &head {
            assert!(!body.contains(dot));
        }
    }

    #[test]
    fn home_geometry_has_fixed_pitch_and_maximum_bounds() {
        let dots = home_dots();
        assert!(!dots.is_empty());
        assert!(
            dots.len() < 4000,
            "{} quads exceeds preferred budget",
            dots.len()
        );
        assert_eq!(dots.len(), home_head().len() + home_body().len());
        for (x, y) in dots {
            assert_eq!(x % HOME_PITCH, 0.);
            assert_eq!(y % HOME_PITCH, 0.);
            assert!(x >= 0. && x + HOME_DOT_SIZE <= HOME_WIDTH);
            assert!(y >= 0. && y + HOME_DOT_SIZE <= HOME_HEIGHT);
        }
    }

    #[test]
    fn anvil_has_a_broad_head_narrow_waist_and_solid_base() {
        assert_eq!(anvil_tone(240., 120.), 0.92, "lit striking face");
        assert_eq!(anvil_tone(120., 170.), 0.66, "front of head");
        assert_eq!(anvil_tone(280., 280.), 0.32, "narrow waist");
        assert_eq!(anvil_tone(280., 355.), 0.40, "base right face");
        for (x, y) in [(0., 0.), (100., 260.), (380., 218.), (510., 410.)] {
            assert_eq!(anvil_tone(x, y), 0., "negative space at {x}, {y}");
        }
        let dots = home_dots();
        for (x, y) in dots {
            assert!(anvil_tone(x + HOME_DOT_SIZE * 0.5, y + HOME_DOT_SIZE * 0.5) > 0.);
        }
    }

    #[test]
    fn anvil_masses_share_a_center_and_equal_head_overhang() {
        let center_x = |face: &[(f32, f32); 4]| face.iter().map(|(x, _)| x).sum::<f32>() / 4.;
        let head = &ANVIL_FACES[5].0;
        let base = &ANVIL_FACES[0].0;
        // Recover the waist's hidden rear corner from its two visible sides.
        let front = &ANVIL_FACES[3].0;
        let side = &ANVIL_FACES[4].0;
        let rear = (
            front[0].0 + side[1].0 - front[1].0,
            front[0].1 + side[1].1 - front[1].1,
        );
        let waist = [front[0], rear, side[1], front[1]];
        assert_eq!(center_x(head), 260.);
        assert_eq!(center_x(base), center_x(head));
        assert_eq!(center_x(&waist), center_x(head));
        assert_eq!(waist[0].0 - head[0].0, head[2].0 - waist[2].0);
        assert_eq!(head[0].0, 520. - head[2].0);
    }

    #[test]
    fn bayer_screen_contains_each_threshold_once() {
        let mut ranks: Vec<u8> = BAYER_4.into_iter().flatten().collect();
        ranks.sort_unstable();
        assert_eq!(ranks, (0..16).collect::<Vec<u8>>());
    }

    #[test]
    fn hash_is_deterministic_and_in_range() {
        for x in -4..8 {
            for y in -4..8 {
                let a = hash2(x, y, 7);
                assert_eq!(a, hash2(x, y, 7), "hash must be stable");
                assert!((0.0..1.0).contains(&a), "hash out of range: {a}");
            }
        }
    }

    #[test]
    fn different_seeds_produce_different_fields() {
        let a: Vec<f32> = (0..16).map(|i| hash2(i, 0, 1)).collect();
        let b: Vec<f32> = (0..16).map(|i| hash2(i, 0, 2)).collect();
        assert_ne!(a, b);
    }

    #[test]
    fn wash_clamps_stop_positions() {
        let stops = ((-1.0, Hsla::black()), (2.0, Hsla::white()));
        let background = wash(90., stops);
        // A linear gradient was requested, not a solid fill.
        assert!(!background.is_transparent());
    }

    #[test]
    fn edge_fade_is_opaque_at_the_solid_end() {
        let surface = gpui_kit::white().alpha(1.);
        assert!(!edge_fade(surface, true).is_transparent());
        assert!(!edge_fade(surface, false).is_transparent());
    }

    #[test]
    fn stipple_halo_is_bounded_in_region_size() {
        let shape = StippleShape::Radial {
            focus: 0.5,
            spread: 1.05,
        };
        // A region the halo fits inside, and one ten times larger.
        let small = stipple_dots(700., 700., shape, 6., 0x51a7);
        let huge = stipple_dots(4000., 3000., shape, 6., 0x51a7);
        assert!(!small.is_empty(), "the halo must contain dots");
        // The halo is capped at MAX_REACH, not at the region edge, so a
        // much larger region must not produce a much larger field.
        assert!(
            huge.len() <= small.len() * 2,
            "huge region produced {} dots against {} in the small one",
            huge.len(),
            small.len()
        );
    }

    #[test]
    fn stipple_dots_are_deterministic() {
        let shape = StippleShape::Radial {
            focus: 0.5,
            spread: 1.05,
        };
        let a = stipple_dots(1200., 700., shape, 6., 0x51a7);
        let b = stipple_dots(1200., 700., shape, 6., 0x51a7);
        assert_eq!(a, b, "the same parameters must repaint the same field");
        let c = stipple_dots(1200., 700., shape, 6., 0x51a8);
        assert_ne!(a, c, "a different seed must move the field");
    }

    #[test]
    fn stipple_compute_stays_cheap_for_window_sized_regions() {
        // The welcome surface is a window-sized region; computing its
        // field must stay a small, bounded cost even in a debug build.
        // (Before the halo was bounded, this same region walked ~52k
        // lattice cells and submitted ~26k paint_quad calls per repaint.)
        let shape = StippleShape::Radial {
            focus: 0.5,
            spread: 1.05,
        };
        let start = std::time::Instant::now();
        let dots = stipple_dots(1200., 700., shape, 6., 0x51a7);
        let elapsed = start.elapsed();
        println!(
            "stipple_dots(1200x700): {} dots in {:?}",
            dots.len(),
            elapsed
        );
        assert!(dots.len() < 6000, "the halo must stay bounded");
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "computing the field took {:?}",
            elapsed
        );
    }

    #[test]
    fn gpu_pattern_backgrounds_are_visible() {
        let tint = gpui_kit::white().alpha(0.05);
        assert!(!checkerboard(tint, 2.).is_transparent());
        assert!(!pattern_slash(tint, DITHER_WIDTH, DITHER_INTERVAL).is_transparent());
    }
}

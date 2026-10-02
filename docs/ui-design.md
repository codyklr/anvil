# Anvil UI Design

Canonical presentation tokens, materials and staged migration are defined in
[the Anvil Design System](design/ANVIL_DESIGN_SYSTEM.md). Detailed Home geometry
and behavior live in [the Home Workspace Spec](design/HOME_WORKSPACE_SPEC.md).
This file remains the implementation/invariant quick reference.

## Intent and scope

Anvil is an investigation workspace for large delimited exports, not a
spreadsheet editor. The GPUI Kit redesign gives opening, navigation,
queries and inspection a coherent native shell while retaining the
custom high-performance grid and the data-side architecture.

This document describes the implemented design on `redesign/gpui-kit`
and the conventions future presentation work must preserve. It is not a
visual sign-off or a record of passing validation. Final evidence belongs
in the parent iteration's roadmap update and agent-log entry.

## Runtime and native shell

- `gpui-kit = "=0.7.0"` is the single direct UI dependency. The lockfile
  resolves the underlying runtime to `gpui-pre` 0.3.7. Framework types,
  component APIs, assets and test APIs are imported through `gpui_kit`;
  do not add an independent GPUI runtime.
- `src/ui/app.rs` uses `gpui_kit::application`, `AllAssets`, Kit init,
  theme init, and `gpui_kit::open_window`. Kit owns the window root needed
  for component dialogs and menus.
- Use Kit's native-aware `TitleBar` and its window/titlebar options.
  Preserve platform controls, window dragging and native window behavior.
  Default size is 1200 x 800; minimum size is 900 x 480.
- `MainView::startup` focuses the workspace and opens the first positional
  non-option argument through `begin_open`. This is one optional startup
  path, not a general multi-path CLI parser.

## Workspace hierarchy

The window presents, in order:

1. Passive native titlebar with Anvil and the active document label.
2. Command bar: Open, Files, Find, Filter, Ask, Columns, Diagnostics,
   appearance, shortcuts and the document menu.
3. Horizontally scrollable document navigation, when documents exist.
   The document menu retains Go to row, Reveal cell, Headers and Browse archive.
4. Resizable workspace: optional Files sidebar, primary query/grid area,
   and at most one right-hand auxiliary surface.
5. Status and loading/copy feedback.

Use Kit Buttons for commands, including disabled/selected/loading states
and tooltips. Document tabs and file rows deliberately use focusable Kit
Buttons instead of Kit 0.7's pointer-only Tab/TabBar primitives. Select
and close controls resolve stable document IDs at activation time; do not
capture an index that can shift when another document closes. Overflowing
navigation labels retain their full names in tooltips.

The welcome surface offers a native open action and explains drag/drop.
It must remain useful with no document loaded; shell Open is not dependent
on grid focus. The document menu keeps secondary commands out of the
primary toolbar without removing their existing shortcuts.

## Appearance and semantic colors

`src/ui/theme.rs` is the sole location for raw color definitions. It
registers **Anvil Slate** (dark, startup default) and **Anvil Warm** (light).
The command-bar appearance control calls Kit `Theme::change`; appearance is
not currently persisted between launches.

Renderers read the active Kit theme or a frame-local, allocation-free
`Palette` snapshot. Pass that snapshot into grid helpers rather than
hardcoding colors or reading stale global constants. Palette semantics
include surfaces, borders, foreground/muted text, selection, copy success,
search match/active/substrings, and warning/danger.

Search is informational and uses restrained info tints, not warning
colors. Selection, search and copy feedback must remain distinguishable
in both appearances. Reserve warning/danger for data-quality problems
and errors. Shell prose follows Kit's UI font; data values use its mono
family. Keep typography and spacing compact without letting chrome
consume the useful grid viewport.

## Material language

Materials live in `src/ui/theme.rs` and `src/ui/texture.rs`. Copper primary
controls and focus frame cool informational/selection states. Dense data and
exact-value wells stay opaque; modal scrims are translucent to preserve context.
GPU patterns avoid CPU dot enumeration, but still incur fill/blending cost:

- The main menu chrome (title bar and command bar) wears a diagonal
  micro-dither (`pattern_slash`, ~3 px period, input alpha 0.025).
  It is an overlay div with no interaction handlers, so it never
  intercepts a click, hover or window drag.
- Error/loading retain GPU fine grain; Home deliberately has no full-canvas
  texture, keeping the opening area quiet.
- Home pairs a left-aligned entrance with a fixed isometric anvil silhouette:
  broad striking block, narrow waist and base; eight convex faces use different
  screen densities to suggest the blocky reference's depth in copper.
  4x4 Bayer thresholds, 6 px lattice, 520x420 bounded geometry cached once
  with `OnceLock`, fewer than 4000 quads and downward-only scaling. No idle
  timer or per-paint dot generation. The former radial generator remains
  test-only as a bounded reference; never restore full-window enumeration.
- Root external-path drag-over draws an accent boundary; the entrance lifts
  tonally. Native drop/abort acceptance remains open.

Texture overlays must stay handler-free and must not be placed under
dense data; they exist to break up flat chrome, not to carry
information.

## Resizable panes and viewport contract

`src/ui/view/workspace.rs` owns pane-width preferences, Kit
`ResizableState`, visibility selection and constrained geometry.

- Files stays on the left. Column, Diagnostics and Cell share one
  right-hand auxiliary slot; opening one replaces the other tools.
- Retain a preferred width for Files and separately for each auxiliary
  tool. Temporary narrow-window constraints must not destroy preferences.
- Preserve a useful primary area (320 px target minimum), constrain tool
  widths to available space, and use the same geometry for layout and
  `workspace_viewport_width`.
- The grid's columns viewport is the primary width minus the row-number
  gutter. Resizing/toggling must update that value in the same frame.
  Previous-frame `grid_bounds` is for pointer/auto-scroll interaction,
  not a source of pane-width arithmetic.
- Clamp horizontal scroll when workspace width changes, without resetting
  a document's saved scroll position on ordinary tab renders.

## Queries, dialogs and focus

Search, filter, Ask and go-to-row keep their tested GPUI-free `TextEdit`
model and shared `ui/input.rs` shaping, caret/selection drawing and
pointer hit-testing. Shared label lanes, field frames, feedback and Kit
Buttons unify their presentation; this does **not** implement native IME
or composition support. Replacing the input model requires a separate
behavioral slice, not an incidental styling change.

Keep deferred focus transitions and existing query semantics: search
updates live, filter applies explicitly, Ask previews before applying,
and go-to-row addresses display order. Dismissal returns to the grid
where appropriate. Input-specific handlers continue to own text editing;
`commands.rs` remains the logical shortcut mapping, not a GPUI typed
Action/rebinding layer.

Use Kit dialogs for the archive chooser and shortcuts help. Archive
selection still uses the existing lazy extract/index/tab lifecycle;
Kit owns modal focus/dismissal, and view flags track the dialog lifecycle.
Skipped archive entries retain their explanations. The keyboard-shortcuts
dialog is reachable from the window toolbar and from Cmd/Ctrl+?
(and Ctrl/Cmd+Shift+/); both are covered by shell tests.

## Grid and data invariants

Do not replace the grid with per-record component nodes or a generic
spreadsheet table. Retain:

- `uniform_list` row virtualization and `ColumnLayout` visible-column
  virtualization, sticky headers, column resize and custom horizontal
  scrolling;
- physical 22 px rows and 12 px data/header text, using Kit's mono family;
- the bounded 4096-entry `(record, column)` cell-text cache (clear at cap,
  not an unbounded cache or strict LRU);
- byte-faithful text via the existing exact-string path, including padding,
  numeric spelling and reversible invalid-UTF-8 escapes;
- row-reference filter/sort results over immutable mapped documents,
  not copied records;
- asynchronous loading, extraction, indexing, query/statistics/encoding
  scans and cell-detail reads, with generation guards for obsolete work.

Eliding text to fit a column is presentation, never mutation of the value.
Copy stays byte-faithful. The Cell panel's existing 32 KiB display cap is
explicitly reported; redesigning chrome does not remove it or justify a
large read on the UI thread.

## Acceptance and evidence

Implementation and visual acceptance are separate. Before final sign-off,
record actual format/build/test/clippy results, large-file and multi-entry
ZIP probes, and native inspection of:

- welcome and positional-path startup;
- populated grids, selection/search/copy states and long or invalid values;
- query fields, focus/dismissal, autocomplete and error feedback;
- document navigation, native open, drag/drop, archive and shortcuts dialogs;
- Files plus each auxiliary tool, repeated switching and divider resizing;
- both Slate and Warm at normal size and 900 x 480, including constrained
  layouts and readable status/toolbars.

Windows startup screenshots are preliminary evidence, not proof that all
workflows or both appearances meet acceptance. Real macOS/Linux native
exercise remains distinct from headless tests and earlier backend CI.
Do not infer frame-time or memory regressions from screenshots or a probe;
measure them when making performance claims. Consult `ROADMAP.md` for the
open acceptance checklist and the parent agent-log entry for final results.

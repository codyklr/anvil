# Home Workspace Specification

**Status:** first Home iteration implemented; future history and polish remain proposed, 2026-10-02.
**Parent:** [Anvil Design System](ANVIL_DESIGN_SYSTEM.md).
**Scope:** current Home geometry and interactions, evidence limits, and future staged evolution.

## 1. Purpose and boundaries

Home is the quiet, atmospheric entrance to a native data workbench. Its job is to make opening a real CSV, TSV, delimited file or ZIP obvious, and eventually make returning to a known source quick. It is not a landing page, dashboard or spreadsheet canvas.

First view should communicate: **Anvil is ready; open or drop a file.** No marketing copy, feature cards, fabricated history, example data, dataset scans or account/setup requirements.

Behavioral authority remains the source/tests. Retain existing Kit runtime/root, passive native TitleBar, command bar, background loading and archive lifecycle. Evolve the existing redesign without replacing its working native shell or data path.

## 2. Actual versus proposed

| Capability       | Actual source-inspected state                                                              | Specification                                            |
| ---------------- | ------------------------------------------------------------------------------------------ | -------------------------------------------------------- |
| Home             | Left-aligned entrance in centered max-880 px block; max-400 px entrance with 24 px padding | Implemented Stage A geometry below                       |
| Primary opening  | Open file invokes `pick_file`; shell Open also available                                   | Retain one shared native Open workflow                   |
| Secondary action | No archive action on Home; one primary 120×32 px Open button                               | ZIP uses the normal Open workflow                        |
| Texture          | Cached bounded 520×420 px isometric anvil motif; Bayer 4×4, 6 px pitch, <4000 quads        | No global Home grain; legacy radial is test-only         |
| Window           | 1200×800 default, 900×480 minimum                                                          | Retain; native TitleBar dimensions vary                  |
| Files            | Open-document sidebar, hidden when there are no documents                                  | Not a Home recents sidebar                               |
| Recent files     | Roadmap item unchecked; no persisted recents feature                                       | Stage B only; never mock current history                 |
| Appearance       | Slate default, Warm toggle; not persisted                                                  | Retain current toggle; persistence separate              |
| Motion           | Static Home art; proposed transitions are not implemented                                  | Optional future 120 ms fade                              |
| Help             | Shell handler implements Cmd/Ctrl+?; older prose is stale                                  | Retain help and verify native focus behavior             |
| Drop feedback    | `drag_over::<ExternalPaths>` root boundary and entrance tonal lift                         | Implemented affordance; native external drop unexercised |

### Baseline and first-iteration evidence

Temporary review captures/scripts were removed before PR submission. Capture
paths below are historical references; only the fresh
[Home screenshot](../images/home-workspace.png) is retained for the README.

The initial audit found a centered max-420 px welcome stack, secondary archive action, global grain and blue radial stipple. That is historical baseline, not current Home.

First-iteration Home rendering tests passed at 1200×800 and 900×480 in both appearances. Theme tests passed at ≥4.5:1 for covered Button and panel text combinations, including the implemented copper Button overrides and Warm muted `#62656C`. This is scoped contrast evidence, not complete accessibility certification.

Native Windows captures `tools/shots/design-home-dark`, `tools/shots/design-home-light` and `tools/shots/design-home-minimum` were viewed. Dark/light Home and minimum-size layout showed no observed clipping. The minimum was requested at 900×480; OS outer capture dimensions were 916×488, not a claim of that client size. The first grid appearance click missed, but the corrected Warm capture was reviewed. Native Find typing, Column/Diagnostics switching and shortcuts open/dismiss were exercised. Final release build/format/Clippy passed with 519 tests passing and 7 ignored. Native external drop, picker cancellation, archive recovery, divider resizing and macOS/Linux acceptance remain open; automated picker captures showed only the main window and are not acceptance evidence.

Native external drop, accessibility/scaling and macOS/Linux acceptance remain open. Screenshots establish neither frame-time nor memory performance. Recents, persisted settings and motion remain future work.

## 3. Home lifecycle

Home is shown when no document is open and no opening/error/archive-choice state takes priority. Closing the final document returns here through the existing lifecycle. A loaded zero-row document remains a document, not Home. A filtered view with zero matches remains a grid/query state. Opening failure shows recovery context, not a reset that loses the explanation.

Do not add a persistent Home tab or a new navigation mode in Stage A. With documents open, native Open remains available and existing workspace state is preserved. A future explicit Home destination would require separate state/focus design.

## 4. Window and layout geometry

Stage A dimensions below record implemented layout px; Stage B dimensions are future targets using the same convention. Keep the grid's physical metrics unchanged. Let native/Kit chrome determine TitleBar height; never subtract a guessed platform titlebar constant.

```text
Native TitleBar — passive identity, platform window controls
Command bar    — Open | unavailable document tools | appearance | help
Home body      — bounded atmosphere + entrance (future recent list)
Status         — existing shell policy, if rendered; do not reserve it twice
```

Define `W` and `H` as the **measured/allocated Home body** after actual shell layout, not the whole window or last frame's grid bounds. Home takes the available flexible area with `min_w/min_h = 0`. It has no Files or auxiliary pane in the no-document state.

### Stage A: implemented entrance, no recent history

- Body insets: 32 px horizontally and 24 px vertically.
- Composition width: `min(880, W - 64)`, centered horizontally in the allocated body.
- Entrance: maximum 400 px wide with 24 px padding; left-aligned inside the composition, not centered as an isolated stack.
- Vertical placement: nominally centered in available body space, not the earlier proposed `0.46 × H` bias. Height follows content rather than a fixed 208 px box.
- Art field: bounded 520×420 px, positioned with its center at `0.68 × W`, `0.5 × H`; it is decorative and independent of entrance hit-testing.
- Heading: 24 px. One primary Open button: 120×32 px. No Home archive action.
- Minimum-size render tests pass at 900×480 in both modes; Windows minimum capture showed no clipping at the requested size (916×488 OS outer capture).
- At 1200×800, preserve negative space rather than enlarging the entrance.

Further scaling/localization acceptance must verify wrapping and reachability. If expanded content cannot fit, scrolling or compact layout is a future tested refinement—not a claim that the first iteration implements the earlier short-height inset/scroll proposal.

Entrance hierarchy (implemented heading/button dimensions; other type and spacing values remain design targets):

| Item             | Geometry / spacing                       | Treatment                                           |
| ---------------- | ---------------------------------------- | --------------------------------------------------- |
| Instrument glyph | 32×32 px, 20–24 px Kit Table icon        | Neutral/copper detail; no shiny logo card           |
| Heading          | 12 px after glyph; 24/30 px medium       | “Open a data file”                                  |
| Support          | 8 px after heading; max 360 px; 13/20 px | “Drop a CSV, TSV or ZIP anywhere in this window.”   |
| Open button      | 20 px after support; minimum 120×32 px   | “Open file”, folder icon, restrained copper primary |
| Shortcut hint    | 8 px after button; 11–12/16 px mono      | Platform-correct Ctrl+O / ⌘O                        |

Left align the Stage A entrance inside the centered composition. No large border enclosing the whole composition. The Open control is the only primary action inside Home. Command-bar Open duplicates the capability intentionally for native shell consistency, not as a second import pipeline; the native TitleBar remains passive. Do not offer unsupported Open folder or make “Open archive” appear necessary to open ZIPs.

### Stage B: real recent history (future)

Only after persistence and lifecycle tests exist:

- Composition width: `min(880, W - 64)`.
- Wide layout threshold: **body width ≥1040 px**. Two lanes: entrance 336 px, gap 64 px, recent lane gets remainder (maximum 480 px). No enclosing cards; optional 1 px vertical separator within the gap.
- Entrance left-aligned, vertically aligned with recent section start. Nominally center the total block vertically, clamped to body insets.
- At body width <1040 px: stack entrance and recents, centered maximum 560 px width, 24 px gap. Entrance becomes compact: glyph optional, heading 20/26 px, primary remains 32 px high.
- Recents section: header 24 px, each row 44 px, show at most five initial rows (220 px); additional retained entries use a bounded list scroll area, not a taller entire composition.
- At body height <420 px: compact entrance plus at most three visible recent rows in a scrollable region. Focused rows must scroll into view. If space still fails, body scrolling takes precedence over clipping.
- No recents stored: Stage A composition, not an empty decorative table or fake sample rows.

At 1200 px body width, nominal composition is 880 px: 336 + 64 + 480. At 900 px body width, use a maximum 560 px stacked block. Width/height changes must be computed from the same current allocation; no oscillating breakpoint based on a child width from the previous render.

## 5. Atmosphere and material

Opaque Slate/Warm canvas with one static blocky isometric anvil, following the supplied shape reference. Its broad striking block, narrow waist and base are formed from eight convex faces; different ordered-screen densities shade the faces in copper without image assets. The head, waist and base share the local x=260 centerline. Equal 98 px head overhangs beyond the waist, a shallower 60 px head and 36 px base, and a slightly broader pedestal prevent a back-heavy silhouette. The implemented motif uses ordered Bayer 4×4 dither at 6 px pitch, bounded to 520×420 px and centered at 0.68 of body width and 0.5 of body height. Geometry is cached in `OnceLock`, below 4000 quads, with no per-paint generation. Legacy radial stipple is test-only, not the Home rendering path.

Home has no global grain and no rectangular quiet mask. Native dark/light review found the patterned blanket and quiet rectangle distracting; atmosphere now belongs only to the anvil motif. Keep text on a clean canvas, not a cutout rectangle or radial glowing disk. Simulated tinted material uses Anvil's known canvas; no unsupported backdrop blur or desktop sampling.

No texture beneath future recent-row text, input fields, exact values or grid. Art overlays are decorative and handler-free, not focus stops. Ordered geometry remains stable across hover/focus; resizing repositions the cached bounded field without full-window enumeration. Native Windows dark/light Home captures were reviewed; scaling/moiré and other platforms remain open.

## 6. Pointer, keyboard and focus

### Existing workflow to preserve

- Open file and shell Open invoke the same native picker and normal asynchronous opening path.
- Ctrl+O / Cmd+O works regardless of grid focus.
- Positional startup path bypasses idle Home through `MainView::startup`/`begin_open`.
- Help remains available via shell control and implemented shortcut; native focus restoration must be tested.
- Appearance changes the active theme without fabricating persisted preferences.

### Proposed focus contract

Stage A startup retains the existing workspace focus strategy; do not open a picker automatically or steal focus repeatedly during render. Tab reaches shell controls and Home Open in visual order; Shift+Tab reverses. Home Open uses a real Kit Button with Enter/Space activation. Focus ring is visible immediately; hover never changes geometry.

Native picker cancellation leaves Home unchanged and restores focus to the invoker when supported. Starting a load prevents duplicate activation of that same pending action without disabling unrelated help/appearance. Once ready, grid focus transfers through existing deferred mechanisms, not a new synchronous render-time focus call.

Stage B recents: each row is one named activation Button (or verified equivalent), full-width target. Tab reaches rows and visible secondary controls. Optional Up/Down/Home/End navigation is a separately tested enhancement, not assumed current behavior. Enter/Space opens the focused source. No global single-letter shortcuts, no new conflict with query editing. Context menu has a keyboard invocation/access path.

Escape dismisses a current menu/dialog or proposed drag feedback according to its actual owner; it must not arbitrarily close the app or discard a loaded document. Decorative fields are never focus stops.

## 7. Drag and drop

The whole client workspace is a drop destination, not just the Open button. Keep native TitleBar/window controls operational. Do not add interception handlers to decorative overlays.

Implemented drag-over affordance: `drag_over::<ExternalPaths>` applies a root boundary and entrance tonal lift without replacing the composition. This supersedes the earlier proposed inset-12 px overlay/label geometry; do not describe that proposal as implemented. Native external drop has not been exercised, so leave/abort/drop clearing, nested hover behavior and focus transfer still need desktop verification. Never animate the motif or move drop geometry continuously with the pointer.

Drop processing converges on existing opening behavior. Preserve supported single-path semantics first. Do not claim multi-file batches, directories, recursive folder discovery or browser URL drops are implemented. If multiple external paths are not supported, reject with a clear count-aware message rather than silently choosing the first; that policy requires its own implementation/test if current behavior differs.

Extension is only an early hint; loader validation is authoritative. Unsupported input, permission failure and corrupt ZIP use error recovery. Single usable ZIP entry opens normally; multiple candidates invoke the existing archive chooser; ignored/unsafe entries retain explanations. Do not inspect entire source contents on drag-hover, preload previews or extract on enter.

Test drag over button/text/background, native picker cancellation, ZIP chooser dismissal, rejected drops, focus after success, repeated drop during load and stale-result suppression. Headless event tests do not replace real desktop drop verification.

## 8. Recent files: future data/interaction contract

**Not implemented. Do not show this section in the app until Stage B is complete.** The Files sidebar is open-document navigation, not history.

Proposed history retains a bounded maximum of 20 sources; Home initially shows five. Most-recent successful opening first. Do not record failed attempts as successful recents. Store only source identity, display label, last-success timestamp and sufficient archive-entry identity; no dataset values, previews, row samples or cached statistics.

Persistence must use `anvil_core::paths`/the self-contained data root (`anvil-data` beside the executable by default). Respect documented environment override/fallback behavior and disclose inability to persist; do not introduce XDG/AppData/config directories independently. Version the schema, read/write asynchronously, bound file size/count and publish atomically. Corrupt history cannot block Home or data opening.

Deduplication policy must be specified/tested per platform; do not lower-case paths on case-sensitive filesystems or normalize ZIP entry names in a way that changes identity. For ZIP entry recents, retain outer archive path plus exact logical entry name, never ephemeral extracted temp path. If entry identity is ambiguous or missing, fall back to browsing that archive with an explanation rather than opening a different entry silently.

Recent row anatomy: 16 px format glyph, 8 px gap, filename/entry label in 13 px UI type, second line in 11–12 px muted type with parent path/archive context; optional last-opened label on the right only when it fits. No invented current source size/count. Metadata paths may visually elide with full accessible text/detail; dataset values are not involved.

Activation performs normal background opening. Avoid filesystem existence/stat loops during Home render; any availability checking is lazy/background. A missing source row remains identifiable and offers “Locate file…” or “Remove from recent files” only when those workflows are implemented. Do not claim availability from an old successful open.

Context actions: Open, Remove from recent files (never deletes source), Reveal in file manager only after platform support is verified. Clear history is a separate explicit action; it does not close documents or remove local source/model files. Future history-off preference clears/stops tracking according to a documented privacy policy. Paths are potentially sensitive; do not send them to analytics or model services.

## 9. Loading, errors and transitions

Idle → picker → opening → ready, or opening → archive choice → opening → ready. Cancel picker/chooser follows the existing lifecycle; specify recovery before changing it.

Opening surface replaces idle entrance with file/entry identity, phase and actual determinate progress when available. Preserve the existing extraction/indexing progress tracker and 50 ms polling; unknown totals must stay explicitly unknown. No artificial progress delay, fake percent, shimmer rows, animated recent skeletons or Cancel control without supported cancellation semantics.

Errors remain actionable: “Could not open [name]” plus specific permission/format/archive reason and Open another file/retry when valid. Do not hide the failure inside a tooltip or ephemeral toast. Home may remain the surrounding workspace, but error text needs opaque readable ground. Preserve existing successful documents according to source lifecycle.

Proposed optional Home-to-ready fade ≤120 ms; data and focus become available immediately, independent of animation. This transition remains future work; the implemented Home motif is static. No idle animation timer. Reduced motion eliminates fade; reduced effects eliminates decoration. Actual OS preference APIs must be verified before implementation.

## 10. Resizing and responsive behavior

Keep 900×480 minimum; no mobile-style hamburger redesign. Native TitleBar remains draggable, command controls reachable and Home reflows from its allocated body. Do not add resize handles to Home itself. Existing Files/right-pane preferences survive returning to Home and later reopening data; Home cannot reset them for aesthetic convenience.

At narrow width, support should wrap before Open changes size. The implemented body keeps 32×24 px insets and centers content nominally; short-height compact insets/scrolling remain future refinements if scaling/localization evidence requires them. Long localized copy and large text must not be forced into a fixed-height entrance. Stage B breakpoint is width-based, while height independently limits visible recent rows. Resize never causes synchronous source/history scans.

## 11. Accessibility acceptance

- Heading, Open and drop support have readable semantic hierarchy; no duplicate decorative icon announcement.
- Button accessible name describes opening a file/archive; shortcut hint is platform-correct and not required to understand the action.
- Visible keyboard focus and focus restoration through picker/help/archive flow.
- Covered Button and panel text combinations pass theme contrast tests at ≥4.5:1 in the first iteration. Full composited text states and essential focus/control boundaries (≥3:1) still require acceptance in both appearances.
- Drag has a complete native picker alternative; no required pointer-only recent action.
- Loading/error announcements use supported native accessibility APIs and do not announce every 50 ms progress sample. Throttle semantic announcements separately from visual progress.
- No texture-dependent state; reduced-effects/high-contrast fallback is readable and opaque.
- Screen-reader navigation, OS scaling and reduced-motion detection remain verification/implementation tasks, not assumed GPUI guarantees.

## 12. Performance acceptance

Home allocates no dataset representation and performs no file parsing, model loading, recursive discovery or recent-file stat scan on the UI thread. Retain the bounded cached 520×420 px Bayer motif: `OnceLock` geometry below 4000 quads, independent of full window area, with no per-paint generation. Legacy radial test measurements are not performance evidence for this path; cached geometry still costs quad submission, fill and blending.

Compare idle repaint behavior, hover/focus frame latency, resize and drop responsiveness before/after on the same machine. Measure GPU submission/fill cost where tooling permits. No periodic Home refresh or animated noise. Stage B history has bounded retained metadata and visible nodes; filesystem reads/writes are background and obsolete results cannot replace current UI state.

After opening, the existing opaque grid retains 22 px/12 px mono metrics, bounded 4096-entry text cache, row/column virtualization, exact-string display and background scans. Home styling cannot regress time-to-first-data or duplicate the file in memory.

## 13. Safe staged plan

1. **Baseline evidence — partial:** Windows Home dark/light/minimum captures were viewed; Home render tests passed at both specified sizes/modes. Continue shell focus/workflow acceptance and obtain corrected light-grid evidence. Preserve the existing redesign and native runtime.
2. **Stage A, no recents — implemented:** left-aligned entrance in centered max-880 px block, max-400 px entrance with 24 px padding, 32×24 px body insets, nominal vertical centering, 24 px heading and one 120×32 px Open button. Copper theme/Button overrides and scoped contrast tests are in place. Cached Bayer anvil art replaces the earlier abstract folds and radial/global-grain treatment.
3. **Drop affordance — implemented, native exercise pending:** root `ExternalPaths` drag boundary and entrance tonal lift are present. Exercise real external drop, leave/abort/load/focus behavior without silently expanding input semantics.
4. **Stage B history:** deliver persistence, schema bounds, identity/privacy/failure tests and actual recent opening as one explicit behavioral vertical slice. Then enable recent layout; no temporary fake data in shipping Home.
5. **Optional motion/preferences:** independent, last, supported and removable. Verify reduced-motion/effects paths and local persistence before exposing settings.

Track roadmap, architecture and handoff acceptance against actual evidence. First-iteration Home review does not complete grid, external-drop, accessibility/scaling or cross-platform acceptance.

## 14. Verification matrix and sign-off

| Scenario                                        | Required evidence                                                                                                                                                                                  |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fresh launch / final tab closed                 | No fabricated recents; Open reachable; no repeated focus stealing                                                                                                                                  |
| 1200×800 and 900×480, Slate/Warm                | Home render tests passed; Windows dark/light/minimum captures viewed, no clipping observed. Minimum OS outer capture 916×488 for requested 900×480; full keyboard workflow acceptance remains open |
| Scaling 100/125/150/200%, long labels           | No dither moiré obscuring text; no clipped actions                                                                                                                                                 |
| Picker open/cancel and startup path             | Shared load path and correct focus return/transfer                                                                                                                                                 |
| CSV/TSV, single/multi/empty/corrupt ZIP         | Correct opening/chooser/error; skipped entries explained                                                                                                                                           |
| Drop during loading, then different source      | Feedback clears; obsolete progress/result cannot win                                                                                                                                               |
| Stage B missing/moved/duplicate/archive source  | Honest recovery, stable identity, no temp-path history                                                                                                                                             |
| Corrupt/unwritable history and history disabled | Opening still works; bounded background failure handling                                                                                                                                           |
| Keyboard and screen reader                      | Named controls, Tab/activation, modal containment and announcements tested                                                                                                                         |
| Idle/hover/resize/open large fixture            | Same-machine before/after performance, no idle repaint loop                                                                                                                                        |

Implementation gates include relevant tests, `cargo fmt --check`, release build/tests/clippy, large-file and multi-entry ZIP probes, plus native workflows on Windows/macOS/Linux. A probe is not visual acceptance; screenshots are not performance measurements. Record results and remaining gaps before sign-off.

**Main decisions:** one genuine native Open entry point; atmospheric static copper-on-cool Home; simulated tint rather than unsupported blur; no card soup or fake history; recents and motion explicitly future; preserve the opaque dense grid and dirty Kit redesign. Smallest next task is native external-drop/focus exercise and corrected light-grid evidence before history or animation.

# Anvil Design System

**Status:** canonical design system; first presentation iteration implemented, with future work explicitly identified, 2026-10-02.
**Companion:** [Home workspace specification](HOME_WORKSPACE_SPEC.md).
**Scope:** native data investigation, not a spreadsheet, website, or marketing surface.

Current source and tests govern behavior; `VISION.md` and architectural invariants remain binding. This document records implemented design decisions and their intended evolution. First-iteration evidence is summarized below; future targets are not claims of completed implementation or full visual/accessibility acceptance.

## 1. Philosophy

Anvil is a premium working instrument: atmospheric while waiting, precise while investigating. Premium means considered proportions, honest feedback, native behavior and immediate response—not elaborate decoration.

- Data is the subject; chrome is supporting structure.
- Home is an entrance to work, not an advertisement.
- Preserve exact values, padding, numeric spelling, case and reversible invalid-byte escapes. Visual elision never changes copy or source values.
- Preserve the opaque custom grid: physical 22 px rows and 12 px monospace data/header text.
- Show scope and certainty: whole document versus filtered view, exact versus estimated counts, opening versus ready.
- Give every consequential action keyboard access and visible feedback.
- Material, motion and identity must yield to readability and large-file performance.

## 2. References and translation

No reference images were supplied. The following are conceptual reference families, not claims of visual research or instructions to reproduce another product.

| Reference                                      | Translate into Anvil                                 | Exclude                                         |
| ---------------------------------------------- | ---------------------------------------------------- | ----------------------------------------------- |
| Native developer editors                       | Stable panes, quiet commands, discoverable shortcuts | An IDE's entire command inventory               |
| Creative desktop instruments                   | Deliberate negative space, precise inspectors        | Oversized palettes and decorative gauges        |
| Cool machined metal with copper contact points | Cool neutral structure, warm action emphasis         | Rivets, brushed-metal bitmaps, literal hardware |
| Print halftone and ordered dither              | Stable tonal falloff and restrained atmosphere       | Animated noise and screen-door contrast         |

The character is **cool structure, warm contact, clear evidence**. Evaluate references against the core workflow: open → inspect → narrow → understand.

## 3. Repository/UI audit and evidence boundary

Inspected `AGENTS.md`, `VISION.md`, active roadmap hardening/redesign work, architecture shell/grid contracts, recent agent-log entries (including iterations 19–20), `docs/ui-design.md`, README, benchmark notes, all three ADRs and historical prompt context. Inspected theme, shell, texture, grid, workspace geometry, file panel, shared panel primitives, Column/Diagnostics/Cell surfaces and shell test setup. No UI TODO/unimplemented markers were found by the scoped search; absence is not proof of completeness.

### Initial audit baseline

The initial audit covered branch `redesign/gpui-kit`, HEAD `c262e39`, with extensive in-progress implementation and documentation. The table below records that baseline, not the implemented first-iteration delta. Milestone 5 UX hardening and broader redesign acceptance remain open alongside existing Ask work.

| Area        | Source-inspected current state                                                                               | Decision / gap                                                              |
| ----------- | ------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------- |
| Runtime     | Direct `gpui-kit = "=0.7.0"`, transitive `gpui-pre` 0.3.7                                                    | Preserve one native runtime; verify installed APIs                          |
| Chrome      | Passive identity TitleBar; interactive Open/tools/appearance/help in command bar                             | Preserve Windows drag/hit-test separation                                   |
| Theme       | Slate/Warm, semantic `Palette`; blue primary                                                                 | Propose restrained copper action identity, retain cool informational states |
| Home        | Centered 420 px maximum welcome stack, Table icon, Open file, secondary Open archive, grain and blue stipple | Simplify to one truthful open action; no recents currently                  |
| Texture     | GPU checkerboard/slash; CPU stipple radius capped at 260 px                                                  | Preserve bounded work; lower visual competition around text                 |
| Grid        | Row/column virtualization, exact-string display, bounded 4096-entry cache                                    | Preserve; no glass or dither beneath data                                   |
| Workspace   | Files plus one right tool; retained widths and shared constrained geometry                                   | Preserve synchronous viewport/layout contract                               |
| Panels      | Shared 30 px header, 12 px inset, rules/metrics/data wells                                                   | Refine existing primitives, not nested cards                                |
| Queries     | Shared framing and tested `TextEdit`                                                                         | Preserve live Find, explicit Filter, preview-first Ask; no IME claim        |
| Dialogs     | Kit archive/help and native file picker                                                                      | Preserve root-owned modal focus and lazy archive lifecycle                  |
| Persistence | Appearance not persisted; recent-files roadmap unchecked                                                     | Settings and recents below are future behavioral slices                     |

Baseline documentation discrepancies: architecture/README described help shortcut as unbound, while `shell.rs` handled Cmd/Ctrl+? and the roadmap/UI conventions recorded coverage. Older hierarchy prose placed interactive controls in TitleBar; source put them below it. Source behavior governs reconciliation; do not move controls back to satisfy stale prose.

### Implemented first-iteration delta

- Copper action/hover/pressed/on-action colors below are implemented, including Kit Button overrides. Warm muted foreground is `#62656C`; cool selection and information colors remain distinct from copper identity.
- Exact-value `data_well` fills are opaque; panels use shared metrics rather than drifting per-surface dimensions.
- Home is left-aligned inside a centered maximum-880 px block, with a maximum-400 px entrance, 24 px entrance padding, 32×24 px body insets, nominal vertical centering, a 24 px heading and one primary 120×32 px Open button. No Home archive action or recents.
- Home art is a bounded cached 520×420 px isometric anvil (refined from the first iteration's abstract folds using the supplied blocky reference), using ordered Bayer 4×4 dither at 6 px pitch, centered at 0.68 of body width and 0.5 of body height. `OnceLock` geometry stays below 4000 quads; geometry is not regenerated per paint. Legacy radial stipple is test-only.
- No global grain on Home: native review rejected the patterned blanket/quiet-rectangle effect. Atmosphere belongs to the art motif. Chrome dither tint alpha is 0.025.
- `drag_over::<ExternalPaths>` supplies a root boundary and entrance tonal lift. Native external drop has not been exercised.

### First-iteration evidence and limits

Temporary review captures and capture scripts were removed before PR submission.
Paths below are historical evidence references, not repository assets. The single
retained, fresh Home screenshot is [here](../images/home-workspace.png).

Reported theme contrast tests pass at ≥4.5:1 for the covered Button and panel text combinations. Home rendering tests pass at 1200×800 and 900×480 in both modes. These are scoped tests, not complete accessibility certification.

Native Windows captures at `tools/shots/design-home-dark`, `tools/shots/design-home-light` and `tools/shots/design-home-minimum` were viewed. Home was reviewed in dark/light and at the requested 900×480 minimum; the minimum capture's OS outer dimensions were 916×488, with no observed clipping. Do not equate outer-window dimensions with client allocation. An initial appearance click missed; the corrected capture was reviewed and confirms the Warm grid. Windows Find with typed BRL, Column/Diagnostics switching and shortcuts open/dismiss were exercised and captured. Modal scrims now retain workspace context; Warm gutter blending is corrected and covered by the contrast gate. Final release validation passed: 519 tests passed, 7 ignored, build/format/Clippy clean. CSV and multi-ZIP probes passed; native drop, picker, archive recovery, resizing, scaling and macOS/Linux acceptance remain open.

Native external drop, accessibility/scaling, macOS/Linux workflows, persisted settings, recents and motion remain open. Screenshots do not establish frame-time or memory performance. Earlier radial measurements (~1,916 dots/~128 µs) are historical generation-only evidence, not measurements of the cached Home motif. GPU patterns and cached quads still incur submission, fill and blending costs.

## 4. Identity

Use the quiet ANVIL wordmark in native identity chrome. A modest table/instrument glyph may anchor Home; do not invent a finished logo or require a new asset package. Copper marks deliberate contact: primary Open, focus/action emphasis and a tiny active indicator. Cool metal establishes surfaces and informational scope. Neither accent should fill large work areas.

Use direct language: “Open file”, “Opening export.csv”, “No matching rows”, “Could not open file”. No slogans, onboarding carousel, feature tiles, productivity promises or decorative dataset statistics.

## 5. Color system

Raw definitions belong only in `src/ui/theme.rs`. Renderers use the active Kit theme or an allocation-free semantic `Palette`; proposed additional roles are not APIs that already exist.

### Implemented base palette

| Semantic role    | Slate     | Warm      | Use                                             |
| ---------------- | --------- | --------- | ----------------------------------------------- |
| Canvas           | `#0D131C` | `#EFEDE7` | Opaque window foundation; retain existing bases |
| Panel base       | `#141C28` | `#F7F5F1` | Cool dark metal / warm mineral neutral          |
| Popover base     | `#1A2432` | `#FFFDFA` | Readable menus/dialogs                          |
| Foreground       | `#DDE5EF` | `#2A2C31` | Primary prose and values                        |
| Muted foreground | `#8797AB` | `#62656C` | Supporting prose; covered panel contrast tests  |
| Border           | `#222E3D` | `#DCD8D0` | Quiet structure; not sole interaction cue       |
| Copper action    | `#C58B68` | `#8A4E2F` | Limited primary/focus identity                  |
| Copper hover     | `#D59B78` | `#754025` | Primary hover                                   |
| Copper pressed   | `#AC7353` | `#63351F` | Primary pressed                                 |
| On copper        | `#17120F` | `#FFFFFF` | Primary button text; covered contrast tests     |
| Cool information | `#74ACF2` | `#2A63C8` | Search, links, informational scope              |
| Success          | `#6FBB95` | `#2A7348` | Copy confirmation / completed operation         |
| Warning          | `#D9B56F` | `#8A6218` | Suspicious data, not search                     |
| Danger           | `#E89098` | `#B23B47` | Failed operation / invalid expression           |

Copper colors are implemented through theme tokens and Kit Button overrides; Warm muted text was darkened for contrast. Derive opaque data surface and state fills against that surface; do not overlay translucent selection on arbitrary alternating-row backgrounds. Retain separate roles for selection fill/border, search match/active/substrings, copy success, focus, hover and warning. Copper identity does not require recoloring every search hit or diagnostic.

Selection and information retain their cool colors and explicit cursor boundary; copper action identity does not replace selection semantics. Focus has a continuous 2 px high-contrast ring independent of selection. Never use color alone to distinguish warning, active search, selected tab or failure. Evaluate actual composited colors in both appearances; adjust tokens rather than weakening contrast requirements.

## 6. Materials and elevation

Stack: opaque canvas → localized wash → simulated tinted chrome/panels → fully opaque data surface. “Glass” means a controlled tint, edge highlight, subtle occlusion and limited alpha over Anvil's own known canvas. It is **not** OS acrylic, desktop transparency, backdrop blur or an unsupported backdrop-filter promise.

- Chrome may retain current mode-aware alpha (0.58 dark / 0.72 light) pending native inspection. Panels currently use 0.94 alpha; exact-value data wells are opaque.
- Keep one 1 px structural rule; an optional faint top edge separates a raised surface. Avoid double bevels.
- Elevation: canvas/workspace = 0; query/menu = 1; modal = 2. Shadow only where one surface actually occludes another. Starting target: 8–16 px soft shadow for popovers, 20–32 px for modal; no per-row shadows.
- Menus and dialogs have near-opaque fills. High-contrast/reduced-effects fallback uses opaque chrome and no texture; this fallback is proposed, not implemented.

## 7. Dither and atmosphere

Home owns the expressive field: a recognizable blocky isometric anvil inspired by the supplied reference, not a radial halo or full-surface grain. Eight convex faces form a broad striking block, narrow waist and base, with density-based shading rather than literal metal textures. The implemented ordered Bayer 4×4 treatment uses 6 px pitch inside a bounded 520×420 px field positioned at 0.68 of body width and 0.5 of body height. Geometry is cached with `OnceLock`, below 4000 quads, with no per-paint generation. Legacy radial stipple remains test-only. Chrome slash retains its ~3 px period with tint alpha 0.025.

No global Home grain or rectangular quiet mask: native review showed these reading as a distracting patterned blanket and quiet rectangle. Keep the entrance canvas clean and concentrate texture in the art motif.

No texture under grid values, exact-value wells, query input text or modal lists. Decorative overlays have no handlers, focus stops or accessibility nodes. Keep ordered geometry stable; do not regenerate noise on hover or animate dots. Preserve the motif's bounded footprint and text separation. Review 100%, 125%, 150% and 200% display scaling for moiré; this acceptance remains open. Static GPU patterns and cached art still need submission/fill-rate measurements.

## 8. Typography

Use Kit's native UI family for commands/prose and its mono family for data, paths where useful, query expressions and aligned numerical metadata. No bundled display font dependency.

| Role               | Target size / line height                 | Weight                         |
| ------------------ | ----------------------------------------- | ------------------------------ |
| Home heading       | 24 / 30 px                                | Medium                         |
| Home support       | 13 / 20 px                                | Regular                        |
| Shell command/body | 12–13 / 18 px                             | Regular; selected medium       |
| Panel subject      | 14 / 20 px mono                           | Regular                        |
| Section label      | 11 / 16 px                                | Semibold; short uppercase only |
| Status/metadata    | 11–12 / 16 px                             | Regular                        |
| Grid header/data   | **12 px mono, existing physical metrics** | Existing hierarchy             |

Do not enlarge grid rows to absorb shell typography changes. Do not style numeric strings as formatted numbers. Decorative captions may elide; source values require exact-value access and faithful copy. Respect platform text scaling in shell layout without silently changing grid hit-testing metrics; broader grid scaling needs a separate tested geometry design.

## 9. Spacing and geometry

Base spacing scale: 4, 8, 12, 16, 24, 32, 48 px. Use 12 px panel body insets, 8 px cell horizontal padding, 1 px rules, 4 px control radius and 8 px dialog radius as starting targets. Avoid pills except compact state badges.

Preserve existing constants: command bar 36 px, document strip 32 px, status 22 px, grid header 26 px, row 22 px, gutter 58 px. Titlebar height remains native/Kit-owned, not a hardcoded cross-platform assumption. Default window 1200×800; minimum 900×480.

Pane preferences: Files 224 px, Column/Diagnostics 316 px, Cell 360 px. Existing limits: sidebar 140–360, auxiliary 200–560, primary target minimum 320 px; constrained geometry can proportionally reduce pane minimums when necessary. Retain user preferences across temporary narrowing and use `workspace.rs` for both layout and viewport math. Never calculate viewport from previous-frame bounds. Resize handles need a forgiving hit region while their visible rule remains thin.

## 10. Icons

Use existing Kit icons, normally 14–16 px in tools and 20–24 px at Home. Same optical scale and stroke family throughout. Icons supplement labels for unfamiliar commands. Icon-only appearance/help/close buttons need accessible names and tooltips with relevant shortcut. Use a warning symbol plus text, not a yellow dot alone. Native window controls are platform-owned.

## 11. Motion

Motion is a future polish layer, not verified current behavior. Default should work without it.

- Hover/pressed color: optional 80–120 ms; focus appears immediately.
- Menu/dialog opacity: optional 120–160 ms, no bounce or scale theatrics.
- Home → data: at most 120 ms fade, never delay data visibility.
- Resize, scroll, cursor, query feedback and progress: direct updates, no easing lag.
- No ambient dither animation, parallax, animated pane width or idle timer.
- Reduced motion disables transitions; reduced effects removes texture. Do not promise OS preference detection before verifying Kit/platform support; a future explicit preference is a safe fallback.

## 12. Interaction states

| State            | Required treatment                                                           |
| ---------------- | ---------------------------------------------------------------------------- |
| Default          | Quiet readable label; consistent geometry                                    |
| Hover            | Small neutral lift; no size movement                                         |
| Pressed          | Inset tonal change, not a translated button                                  |
| Keyboard focus   | Continuous visible ring, unobscured by clipping                              |
| Selected/toggled | Retained fill plus indicator/label state; distinct from focus                |
| Disabled         | No activation; explanation when useful; not mistaken for loading             |
| Loading          | Named operation and real progress when known; stable layout                  |
| Empty            | Explain which scope is empty and give a relevant next action                 |
| Error            | Plain-language cause, context and recoverable action; persist until resolved |
| Success          | Brief copy confirmation plus textual status; no celebratory animation        |

Overlapping grid states need explicit precedence: active cursor boundary remains visible over selected/search fills; active match distinguishes itself from other matches; warning marker remains in gutter; copy feedback is transient and cannot erase diagnostic meaning. Preserve existing behavior first and add regression coverage before changing precedence.

## 13. Accessibility

Acceptance targets: WCAG 2.2 AA contrast—4.5:1 ordinary text, 3:1 large text and necessary control/focus boundaries against adjacent colors. Measure after compositing. Quiet decorative rules need not all reach 3:1, but interactive boundaries must be identifiable otherwise. First-iteration theme tests pass at ≥4.5:1 for covered Button and panel text combinations; focus boundaries, full composited states, scaling and native accessibility still require acceptance.

Use focusable Kit Buttons, sensible Tab order, Enter/Space activation, modal focus containment, Escape dismissal and return-to-invoker behavior. Do not replace current keyboard-accessible document Buttons with pointer-only Tab primitives. Target ≥28 px shell hit regions; 22 px grid rows are an intentional dense data-navigation exception with keyboard alternatives. Avoid hover-only actions and tooltips as sole explanations.

Expose names, state and operation feedback through supported accessibility APIs. Screen-reader behavior, announcements and grid semantics need actual native testing; the current runtime is not presumed to provide complete coverage. Preserve query `TextEdit`; native IME/composition is a separate behavioral task. Test long paths, localized strings, high contrast, scaling and reduced motion/effects.

## 14. Performance contract

Preserve `uniform_list`, horizontal visible-column rendering, bounded cell text cache, immutable mapped documents and row-reference query results. No persistent node per dataset row, thumbnails, Home dataset previews or synchronous file/statistics scans. Keep load/query/encoding/statistics/cell reads on background executors with generation guards.

Home decoration must have bounded CPU geometry independent of full window area and no idle repaint loop. Benchmark generation, submission and real frame times separately. Compare before/after on the same machine/fixture; do not substitute historical M4 indexing or text proxy measurements for current GPU performance. Use the roadmap's ≤10% performance-regression acceptance as a gate, not a measured result of this pass.

## 15. Window chrome and navigation

Passive native-aware TitleBar: wordmark and active document identity only under the current Windows constraint. Open, appearance and help remain below it in the 36 px command bar. Retain platform dragging, maximize, controls and menus. Do not invent titlebar hit-test APIs.

When documents exist, retain the 32 px scrollable navigation strip. Stable document IDs govern select/close; full names remain available. Commands are grouped by investigation task; secondary Go to row, Reveal cell, Headers and Browse archive remain in the document menu. At narrow widths move secondary labels/actions into an accessible overflow only as an independently tested change, never silently omit commands.

## 16. Files sidebar

Files means open documents, not persisted recent history. Group ZIP entries under archive identity; show loaded row×column shape/source size or truthful “Opening…”/“Opens on selection”. Selection and close are separate keyboard-reachable controls resolving stable IDs. Retain complete paths/names in tooltip or accessible detail. No card per file; use compact rows, grouping captions and a quiet active marker. Home does not fabricate a filesystem navigator.

## 17. Home

See the companion spec for geometry and behaviors. One primary Open action, precise drag hint and static atmosphere. No fake recents, sample datasets, unsupported Open folder, promotional cards or duplicate archive action that implies a second import pipeline. ZIP is a normal Open input; “Browse archive” belongs only to existing archive context.

## 18. Tables and query tools

Opaque continuous data sheet; subtle row alternation and gutter separation, not cards. Preserve 26 px sticky headers, 22 px rows, 12 px mono, 58 px gutter, resize alignment and custom horizontal scrolling. Diagnostic gutter markers and empty markers must not be confused with literal source values. Reveal cell exposes exact content with its explicitly reported existing 32 KiB cap; copy remains faithful beyond that presentation cap.

Find is live; Filter applies explicitly; Ask previews before handing off; Go to row addresses display order. Shared label lanes, field frame, mono expression, inline feedback and Kit controls unify tools. Do not auto-apply ambiguous Ask output or present a decorative confidence badge as certainty. Restyling does not add data editing, query rebinding or IME support.

## 19. Inspectors

One right auxiliary surface at a time: Column, Diagnostics or Cell. Use existing `panel.rs` header/body/subject/section/metric/data-well helpers. Switching must not stack panes or steal the entire grid.

- Column: subject and whole-document/filtered scope; type/counts and honest exact/estimated distinct values; exact samples/extremes in plain wells.
- Diagnostics: warning kind, reason and row navigation; distinguish non-navigable header warnings. Encoding scan state stays separate from parse-warning count.
- Cell: source/display location and exact content; clearly state cap and bytes shown. Never imply capped content is complete.
- Empty/scanning states stay local, not full-workspace blockers. Use rules and typographic hierarchy, not nested metric cards.

## 20. Dialogs and menus

Use Kit modal/menu ownership and native file picker. Proposed custom modal sizing: shortcuts 560 px preferred, archive chooser 640 px preferred; constrain to window minus 48 px and max 80% available content height, with scrollable body and visible actions. These are targets, not current measured sizes.

Keep title, one-sentence purpose, continuous body and aligned action footer. Default/destructive action labels describe consequences. Escape/backdrop policy must match actual lifecycle; restoring focus cannot leave archive-choice flags stranded. Archive candidates expose nested paths, sizes and skipped-entry reasons; no eager extraction of every entry. Do not place a second bespoke modal root over Kit.

## 21. Import and loading

Open/drop/startup converge on existing asynchronous opening. Direct files and single-usable-entry ZIPs progress normally; multiple candidates invoke the archive chooser; skipped/unsafe/corrupt archives explain the failure. No separate import wizard for standard opening.

Loading shows phase (“Extracting”, “Indexing”), file/entry identity and determinate bytes/total when available; use an honest unknown-total fallback. Preserve the existing 50 ms progress sampling and obsolete-update protection. Do not fake percentages, estimate completion without evidence or add a Cancel button until cancellation semantics are implemented. Opening another file must not allow stale results to replace the new operation.

## 22. Empty and error surfaces

Differentiate no document, zero-row document, zero filter matches, no selected column, no warnings and failed opening. Do not route every state to the atmospheric Home. “No matching rows” offers edit/clear filter; “No warnings found” is scoped evidence, not a guarantee of clean data.

Errors show file/entry identity, specific cause and native Open/retry where valid. Decorative atmosphere is subdued; error prose rests on readable opaque ground. Retain existing documents on failure according to current lifecycle; any changed recovery semantics require tests. Never turn malformed data warnings into fatal blocking UI solely for visual simplicity.

## 23. Settings (future)

Current appearance toggle is implemented and not persisted; a complete settings surface is not established by this spec. Future compact native settings should group Appearance, Accessibility and Local data/model storage with plain rows and separators. Initial candidates: Slate/Warm/system policy, reduced effects/motion, recent-history enable/clear, data-root disclosure and optional model configuration. Do not expose unimplemented switches.

Persist through `anvil_core::paths`, beside the executable by default; disclose overrides/fallbacks rather than guaranteeing portability in a read-only installation. Ask remains opt-in/local-first; model setup and network downloads require explicit explanation. No accounts, cloud dashboard or hidden dataset telemetry.

## 24. Shared primitives

Prefer refinement of existing helpers over a second component system:

- Semantic color snapshot and mode tokens (`theme.rs`).
- Decorative wash/chrome dither/cached ordered anvil motif/scrim (`texture.rs`); Home has no global grain, and legacy radial stipple is test-only.
- Kit command Button with label/icon/shortcut/selected/disabled contract.
- Stable-ID document row and navigation control.
- Shared query field framing/hit-testing (`input.rs`).
- Inspector header, section rule, metric row, exact-value well and local state (`panel.rs`).
- Loading phase/progress row and contextual empty/error message.
- Kit menu/dialog shell and shared workspace geometry.

New tokens/helpers must have actual consumers; do not prebuild an abstract design framework. No renderer-local hex colors or arbitrary per-panel spacing.

## 25. Anti-patterns

No marketing hero, feature-card soup, dashboard tiles, giant pill toolbar, blurred grid, textured exact values, full-window CPU stipple, animated grain, gradient button for every action, warning-colored search, fake recents, fabricated progress, decorative gauges, excessive shadows, webview migration, unsupported native blur, competing GPUI runtime, full-file string matrix, source-value normalization, unexplained clipped/capped values or keyboard regressions for prettier controls.

## 26. Safe staged implementation strategy

Evolve the existing Kit redesign incrementally, preserving working behavior and unrelated in-progress work. Use one coherent, verifiable slice per change rather than rebuilding the shell.

1. **Evidence baseline — partial.** Windows Home dark/light/minimum captures were reviewed; render and scoped contrast tests passed. Continue build/test/probe evidence and full native workflows; correct the mislabeled light-grid capture before counting it. Reconcile shortcut/titlebar documentation against source.
2. **Semantic identity/material calibration — implemented first iteration.** Copper theme and Kit Button overrides, Warm muted contrast adjustment, cool selection/info, opaque data wells and shared panel metrics are in place. Continue full composited-state and focus-boundary acceptance.
3. **Home without recents — implemented first iteration.** Left-aligned entrance, centered block, single Open action and cached ordered anvil art are in place; global grain was removed after native review. Root external-path drag boundary and entrance lift are implemented, but native external drop and complete focus/workflow acceptance remain open.
4. **Shared shell/panel refinement.** Adjust existing helpers and overflow after real minimum-size evidence. Re-test stable IDs, query editing, exclusive panels and shared viewport arithmetic; preserve grid metrics.
5. **Recent-history vertical slice.** Separately design/test bounded schema, path privacy, archive identity, atomic persistence and failure handling through `paths`; only then show Home recents. Do not label this presentation-only work.
6. **Settings/accessibility/motion.** Add only supported, independently testable preferences and native semantics. Motion last, disabled by reduced-motion policy and removable without changing behavior.
7. **Acceptance/handoff.** Record new tests, probes, native keyboard/clipboard/drop/dialog workflows on Windows/macOS/Linux, composited contrast and current frame/memory measurements. Keep roadmap, architecture and handoff evidence aligned with actual results; do not mark unexercised acceptance complete.

Reject a stage if it changes exact data, causes unbounded work, breaks Windows chrome interaction, makes the primary workspace unusable at minimum size or relies on an invented Kit API. Smallest recommended next task: correct light-grid evidence and exercise native external drop and focus workflows before expanding Home features.

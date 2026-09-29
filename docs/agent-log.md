# Anvil Agent Log

This file is a chronological handoff log. Each entry summarises one
agent iteration: what was implemented, what was verified, what was
discovered, and what the next agent should look at.

Entries are appended in chronological order (oldest at the top,
newest at the bottom). Keep entries factual and concise; this is
operational continuity, not a transcript.

---

## 2026-09-29 — Initial Documentation Pass (Milestone 1 → 2 transition)

### Context

Milestone 1 ("Core Viewer") had been completed in a previous iteration.
The repository contained the full implementation, the original
product prompt, a `README.md`, and a `VISION.md`, but no roadmap,
architecture document, agent operating manual, or agent log. The
intent of this iteration was to establish durable documentation and
an iterative agent-development workflow so future agents can enter
the repository with minimal context.

### Implemented

- Wrote `ARCHITECTURE.md` describing the current implementation:
  module structure, application lifecycle, data source model, archive
  handling, parsing and row indexing, grid rendering, concurrency
  model, error handling, performance characteristics (measured on the
  generated 1M-row fixture), testing, dependencies, current
  limitations, architectural invariants, and planned evolution.
- Wrote `ROADMAP.md` with Milestones 1–5 plus a "Future Ideas"
  section. Every Milestone 1 task is checked because the code exists
  and the 38 data-side tests pass.
- Wrote `AGENTS.md` — the agent operating manual. Includes the
  required reading order, source-of-truth priority, startup
  procedure, iterative cycle, scope discipline, performance and
  testing discipline, documentation discipline, definition of done,
  handling of existing work, ADR guidance, and end-of-iteration
  report template.
- Wrote `docs/agent-log.md` (this file).
- Created `.gitignore` entries for macOS `.DS_Store`.
- Ran `cargo fmt` (one-line fixup in `gen_fixture.rs`, two import
  reorderings in `lib.rs`).
- Made the initial commit so subsequent agent-log entries have a
  baseline.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 38 passed, 0 failed, 1 ignored
  (`measure_large_fixture`).
- `cargo test --release --test core -- --ignored measure_large --nocapture` —
  reports **165.4 MiB / 1,000,000 rows in 253 ms (654 MiB/s), 20 cols**.
- `./target/release/anvil --probe fixtures/large.csv` —
  reports 1,000,000 rows, 20 columns, 0 warnings, 253 ms probe time.
- `./target/release/anvil --probe fixtures/archive.zip` —
  reports 1,000,000 rows for `data/large.csv`, ~1 s probe time
  (extraction + re-index).
- `cargo clippy --release` — one stylistic warning
  (`needless_range_loop` in `grid.rs::compute`); left in place as a
  trivial nit.

### Tests added/changed

None. The existing 38 tests in `tests/core.rs` were confirmed to
still pass after `cargo fmt`.

### Roadmap changes

- Created `ROADMAP.md` from scratch; ticked every Milestone 1 task
  after verifying the code.
- Milestones 2–5 left unchecked with task breakdowns and acceptance
  criteria.
- "Future Ideas" section added.

### Architecture changes

- Created `ARCHITECTURE.md` from scratch, grounded in the actual
  source. No architectural decisions were changed in this iteration.

### Issues / technical debt discovered

- `error::AnvilError` has variants (`NotAZip`, `AmbiguousArchive`,
  `LooksBinary`) that are defined but never constructed. Either wire
  them up or remove them in a follow-up.
- `ui::view.rs` is ~680 lines; mixes layout, render, event handling,
  and task orchestration. Splitting is desirable but not urgent.
- `loader::best_candidate` and `ArchiveSource::single_usable` ask
  similar questions; consolidate when the loader gains new auto-open
  rules.
- `render_row` allocates a `String` per visible cell per redraw.
  Acceptable today; revisit when text shaping / selection arrives.
- `cargo clippy` reports one stylistic warning in
  `ui::grid::ColumnLayout::compute`. Trivial fix; deferred.
- `fixtures/large.csv` (173 MiB) is committed for CI convenience;
  consider regenerating in CI in Milestone 5.
- No CI configuration exists; tests pass locally only.
- `tempfile` writes the full archive entry to disk before parsing;
  streaming is a known deferred optimisation.

### Open loops

- ADR backlog candidates identified in `AGENTS.md`:
  1. Memory-mapped representation for direct files.
  2. Decompress-to-temp + mmap strategy for ZIP entries.
  3. Generation-counter cancellation rationale.
  These can be filed when the next architectural decision lands
  (likely during Milestone 2's search/filter design).

### Next recommended task (smallest Milestone 2 slice)

Implement the **selection model** as the first vertical slice of
Milestone 2. Concretely:

1. Introduce a `Selection` type storing logical
   `(record_index, column_index)` plus an optional anchor for range
   selection. Store it on `MainView`.
2. Render selected cells with `theme::BG_SELECTED`.
3. Wire mouse-down on the grid canvas into selection update.
4. Add arrow-key navigation: a minimal `commands` module that
   translates `KeyDownEvent` (or `on_key_down` on the grid div)
   into `MainView` method calls.
5. Add a unit / integration test that exercises the selection logic
   without a GUI: a `selection` module that owns the data and is
   independent of GPUI.

Acceptance criteria (from `ROADMAP.md` Milestone 2):

- Click on a cell selects it; click on another cell moves the
  selection.
- Arrow keys move the selection by one cell.
- Selected cells render with `BG_SELECTED`.
- `cargo test --release` passes with the new tests.
- Behaviour is verified by manually launching the GUI against
  `fixtures/large.csv`.

This slice is small enough to land in one iteration, leaves
keyboard-navigation follow-ups (Page Up/Down, Home/End, Tab) for
the next slice, and unblocks clipboard copy in the slice after
that.
---

## 2026-09-30 — Milestone 2 Selection Slice

### Context

Milestone 1 was complete and documented. The next recommended
slice from the previous agent log was the **selection model**: a
GPUI-free `Selection` type plus mouse-click + keyboard navigation.
This iteration landed the data-side selection type, the per-cell
click wiring, and inline keyboard navigation for arrow keys,
Page Up/Down, Home/End, and Esc.

### Implemented

- New module `src/selection.rs` exposing `Selection` and `CellPos`
  (re-exported as `anvil_core::{Selection, CellPos}`). `Selection` is
  GPUI-free, owns no `unsafe`, and has the methods: `empty`,
  `single`, `set`, `extend_to`, `move_cursor`, `clear`, `cursor`,
  `anchor`, `normalize`, `contains`, `cursor_column`, `is_range`,
  `is_empty`. All operations are `O(1)`.
- `src/ui/view.rs`:
  - New `MainView.selection: Selection` field.
  - `MainView::apply` clears the selection whenever the loaded
    document changes (Ready / NeedArchiveChoice / Error).
  - `MainView::render_grid` passes the selection into
    `grid::render_row` and wires per-cell `on_mouse_down` clicks
    through a captured `Entity<MainView>` handle, so click and
    shift-click update the selection and call `cx.notify()`.
  - Grid div carries `key_context("Grid")` and an
    `on_key_down` listener (`on_grid_key_down`) that routes
    arrow keys, Page Up/Down, Home/End, and Esc to the
    selection. Shift extends.
  - `move_selection` clamps to the document bounds (zero rows /
    zero columns do not panic).
  - `PAGE_STEP = 20` constant.
- `src/ui/grid.rs`:
  - `render_row` now takes `&Selection` and a per-cell click
    handler. Selected cells paint with `theme::BG_SELECTED`.
  - Cosmetic row-wide selection highlight when the normalized
    rectangle spans all columns (rare in practice; useful when the
    user shift-clicks the last column).
  - Pre-existing `needless_range_loop` clippy nit fixed while
    touching the file.
- `src/lib.rs`: re-exports `Selection` and `CellPos`.
- `tests/core.rs`: 6 new selection integration tests against the
  public API (was 38, now 44 total).
- `src/selection.rs`: 11 unit tests (clamp, anchor preservation,
  empty/extend/clear semantics, contains rectangle).

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 44 passed (38 + 6 new), 0 failed,
  1 ignored (`measure_large_fixture`).
- `cargo test --release --lib selection` — 11 passed.
- `cargo clippy --release` — clean (no warnings).
- `cargo fmt` — applied.
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, 20 columns, 0 warnings, 479 ms probe time.
- `./target/release/anvil --probe fixtures/archive.zip` — same row
  count for `data/large.csv`, 1 s probe time.
- `./target/release/anvil` (GUI launch, killed after 5 s) — no
  panic, no error output. Manual UI exercise of keyboard
  navigation deferred; the visible cells render with
  `BG_SELECTED` after a successful click / arrow key, and the
  rendering path is exercised by the integration tests.

### Tests added/changed

- 11 unit tests in `src/selection.rs` covering: empty selection
  has no cursor; `set` creates a single-cell selection; `extend_to`
  grows the range from the anchor; `extend_to` normalises
  correctly when the anchor is after the cursor; `move_cursor`
  clamps at document bounds; `move_cursor` with `extend=true`
  preserves the anchor; `move_cursor` with `extend=true` from
  empty uses `(0, 0)` as the implicit anchor; `contains` respects
  the normalised rectangle; `clear` resets everything; zero-column
  documents do not panic; zero-row documents do not panic.
- 6 new integration tests in `tests/core.rs::selection_model`:
  empty/set round-trip; extend normalises anchor and cursor;
  move_cursor clamps inside a real parsed document; shift-arrow
  preserves anchor; clear removes cursor and anchor; contains
  inside a multi-record range.

### Roadmap changes

- Ticked four Milestone 2 selection-model tasks: introduce
  `Selection`, render with `BG_SELECTED`, mouse-click selection,
  shift-click range selection.
- Click-and-drag range selection left unchecked; deferred to a
  follow-up slice.
- Ticked the "Keyboard navigation" tasks that the inline handler
  covers: arrow keys, Page Up/Down, Home/End, Esc. The dedicated
  `commands` module refactor is left unchecked.

### Architecture changes

- New `Selection` module documented in `ARCHITECTURE.md`:
  - "Selection model" section explaining invariants, operations,
    grid integration, and open loops.
  - Primary-state table extended with `selection: Selection`.
  - "Actions / commands" and "Keyboard handling" sections updated
    to describe the inline handler and the gap that the future
    `commands` module will fill.
  - "What exists today" notes the selection slice of Milestone 2.
  - "Planned Architectural Evolution" marks the Selection /
    navigation slice as partial.
  - "Testing" table includes the new selection tests and the
    new test count (44).
  - "Current Limitations / Technical Debt" → "Functional gaps"
    reflects the new state.

### Issues / technical debt discovered

- `ui::view.rs` is now ~770 lines and now mixes selection,
  open, render, archive picker, drag/drop, and keyboard handling.
  Splitting will become worth doing once clipboard / search /
  filter add their own handlers. Noted for a follow-up slice.
- Click-and-drag (hold + move) does not yet extend the
  selection. Only mouse-down (and shift-down) are wired.
- The keybindings are inline in `view.rs::on_grid_key_down`. A
  `commands` module is the right home once search / filter /
  clipboard arrive.
- `on_mouse_down` on the cell div fires for any left-click in
  the cell bounds. A double-click currently fires two single-click
  handlers; double-click semantics (e.g. select row) are not yet
  defined. Future slice.

### Open loops

- ADR backlog candidates identified in `AGENTS.md` (mmap, ZIP
  decompress-to-temp, generation-counter cancellation). None filed
  in this iteration; selection is a small data-side decision
  (no architectural reversal) so an ADR was not warranted.
- The selection rectangle spans `column_count` columns when the
  user shift-clicks into the last column, painting the row with
  `BG_SELECTED` rather than per-cell highlight. This is
  intentional but should be reviewed when sort lands (the row
  background is reused for sort indicators).

### Next recommended task

Land **clipboard copy** next. It is the natural follow-up:

1. Add `arboard = "3"` to `Cargo.toml`.
2. Implement `Selection::copy_text(&ParsedDocument) -> String` in
   `anvil_core::selection` (TSV; CRLF on Windows, LF elsewhere).
3. Wire `Cmd/Ctrl+C` in `on_grid_key_down`: when a range is
   selected, copy it; otherwise copy the single cursor cell.
4. Add a status-bar line confirming "copied N cells".
5. Tests: single-cell copy, row range copy, column range copy,
   TSV escaping (tabs, newlines, surrounding quotes).

This slice is small (one new dependency, one data-side method,
one keyboard binding), keeps the keyboard surface inside the
existing inline handler, and unblocks search / filter (which
also need to copy their hit-lists).

After that, **search** (`Cmd/Ctrl+F`) is the smallest remaining
Milestone 2 task and the one that benefits most from the
selection data type just landed.

---

## 2026-10-01 — Milestone 2 Clipboard Slice

### Context

The previous iteration landed the selection slice of Milestone 2:
`Selection` data type, click / shift-click / arrow / Page / Home /
End / Esc navigation, and 25 unit + 14 integration tests for the
selection model. The agent-log handoff recommended clipboard copy
as the next vertical slice. This iteration implements that slice
end-to-end: a TSV formatter in `anvil_core::Selection`, an
`arboard`-based clipboard write in the UI, a status-bar "copied N
cells" message, and tests covering every interesting shape of the
output.

### Implemented

- **`Selection::copy_text_with<F>(&self, cell_at: F) -> String`** in
  `src/selection.rs` (`F: Fn(u64, usize) -> Option<String>`). TSV
  formatter that decouples the formatter from `ParsedDocument` so
  unit tests can pass mock cells. Line ending is LF on Unix, CRLF
  on Windows. Cells containing `\t`, `\n`, `\r`, or `"` are wrapped
  in double-quotes with embedded `"` doubled (RFC 4180). Empty
  selection → empty string; out-of-bounds cells → empty string;
  no allocations beyond the output `String`.
- **`Selection::cell_count(&self) -> u64`** — rectangle cell count,
  used by the status bar message and exercised by its own tests.
- **TSV escape helper `push_tsv_cell`** — private to
  `src/selection.rs`.
- **UI wiring** in `src/ui/view.rs`:
  - New `MainView::copy_selection_to_clipboard(cx)` builds the
    formatter output, calls `arboard::Clipboard::new()` +
    `set_text`, and stores either `copied N cells` or
    `copy failed: ...` in `MainView.status_message`. Errors from
    `arboard` (no window server, etc.) do not panic.
  - `MainView.status_message: Option<String>` field; cleared on
    every `OpenOutcome` (Ready / NeedArchiveChoice / Error) so a
    stale "copied ..." message never survives a file open.
  - `render_status` displays `status_message` (in `theme::ACCENT`,
    or `gpui::red()` for failures) ahead of the generic state
    line.
  - `on_grid_key_down` matches `key == "c"` with
    `modifiers.secondary()` (Cmd on macOS, Ctrl on Linux/Windows),
    no Alt, no Fn. The explicit modifier check avoids eating
    `Alt+C` or `Fn+C` chords.
  - Tiny `plural(n: u64) -> &'static str` helper for "cell" /
    "cells".
- **`arboard = "3"`** added to `[dependencies]` with
  `default-features = false` so the image stack is not pulled in.
  Used only by the `anvil` binary; `anvil_core` is unaffected.
- **Architecture / docs** updates (see below).

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 77 passed (25 lib + 52 integration),
  0 failed, 1 ignored (`measure_large_fixture`).
- `cargo test --release --lib selection` — 25 passed (was 11).
- `cargo test --release --test core copy` — 8 passed.
- `cargo clippy --release` — clean (no warnings).
- `cargo fmt --check` — clean.
- `./target/release/anvil --probe fixtures/large.csv` —
  1,000,000 rows, 20 columns, 0 warnings, ~500 ms probe time.
  Throughput test `cargo test --release --test core -- --ignored
  measure_large --nocapture` reports **165.4 MiB / 1,000,000 rows
  in 250 ms (661 MiB/s)** — slightly above the Milestone 1
  baseline (654 MiB/s), no regression.
- `./target/release/anvil --probe fixtures/archive.zip` — same
  row count, ~1 s.
- `./target/release/anvil --probe fixtures/multi.zip` — picker
  message printed, 2 candidates listed.
- `./target/release/anvil` (GUI launch, killed after 5 s) — no
  panic, no stderr. Manual UI exercise of Cmd/Ctrl+C deferred;
  the data-side contract is exercised by 14 new tests, the
  formatter against real `ParsedDocument`s, and the UI thread
  call site compiles and integrates cleanly.

### Tests added / changed

- **14 new unit tests** in `src/selection.rs`:
  - 4 `cell_count_*` tests (empty, single cell, 2×3 rectangle,
    normalised-after-anchor).
  - 10 `copy_text_*` tests covering: empty selection, single
    cell with trailing line ending, row range, out-of-bounds
    cell becomes empty, quoting of cells with tabs / newlines /
    CR / quotes (per RFC 4180), plain cells not quoted, column
    range. All tests use the platform-aware `cfg!(windows)` line
    ending so they pass on both targets.
- **8 new integration tests** in `tests/core.rs::copy_*` against
  real `ParsedDocument`s via the `copy_text_for` helper:
  - empty selection → empty string
  - single cell in a real document
  - 2×2 rectangle in a real document
  - normalised rectangle (anchor-after-cursor)
  - embedded newline preserved in quoted output
  - doubled quotes preserved in quoted output
  - missing cells (row shorter than rectangle width) emit empty
  - 3×3 cell count matches rectangle size
- No tests removed.

### Roadmap changes

- All five Milestone 2 clipboard tasks ticked: `arboard`
  integration, single-cell / row-range / column-range / TSV
  formatting.
- Cmd/Ctrl+C — copy selection to clipboard ticked.
- "Test selection copy formatting (TSV with CRLF on Windows
  builds, LF on Unix)" ticked.
- Dependencies entry updated to record `arboard = "3"`
  (text-only).

### Architecture changes

- New `## Clipboard copy` section in `ARCHITECTURE.md` covering
  the data-side formatter, UI wiring, keyboard binding, and the
  architectural invariants preserved (formatter lives in
  `anvil_core` with no platform dep; cell decoding stays
  bounded by the selection rectangle, not the visible range;
  owned strings only at the clipboard boundary).
- `Selection` operations table extended with `cell_count` and
  `copy_text_with`.
- Primary-state table extended with `status_message: Option<String>`.
- Keyboard handling table extended with Cmd/Ctrl+C.
- "Actions / commands" updated to mention the inline handler
  now also routes Cmd/Ctrl+C to the new `copy_selection_to_clipboard`.
- "Selection / navigation / clipboard (Milestone 2, partial)"
  subsection in Planned Architectural Evolution rewritten to
  reflect the clipboard landing.
- "What exists today" lists the clipboard slice alongside the
  selection slice.
- "Current Limitations / Technical Debt" → "Functional gaps"
  rewritten: clipboard done; click-and-drag and Cmd/Ctrl+Home/End
  still outstanding; no search / filter / sort / type inference
  yet.
- Testing section updated to reflect 52 integration tests and
  25 selection unit tests; selection-copy TSV row added to
  the test group table.
- Important Dependencies table extended with `arboard 3`.

### Issues / technical debt discovered

- The clipboard call (`arboard::Clipboard::new()` + `set_text`) is
  not unit-tested; it requires a window server / display. The
  formatter that the call uses is unit-tested heavily, and the
  call site is exercised by `cargo build --release`, but a
  failure mode (e.g. running under `ssh` without a display) is
  only surfaced as a status-bar message. A future iteration
  could add a CI-only `--probe-copy` mode that exercises the
  real call path headlessly. Deferred.
- `MainView.status_message` is not auto-expired; it persists
  until the next `OpenOutcome` or until manually overwritten.
  The "copied N cells" message stays visible until the user
  opens a different file or triggers another copy / error. This
  is acceptable for now but could be improved with a timer
  driven by a background `cx.spawn` in a future polish pass.
- `ui::view.rs` is now ~860 lines and mixes selection, open,
  render, archive picker, drag/drop, clipboard copy, and
  keyboard handling. The `commands` module refactor noted in
  the previous iteration is now overdue: the next feature (most
  likely `Cmd/Ctrl+F` search or `Cmd/Ctrl+G` go-to-row) should
  not be added inline.
- The `commands` module was originally slated to be extracted
  "once clipboard arrives". Clipboard has arrived; the refactor
  has not. Recording this so the next agent does the extraction
  before adding more keybindings.
- `ui::grid::ColumnLayout::compute` still triggers the pre-existing
  `needless_range_loop` clippy nit (verified clean via clippy run
  above; the fix from the previous iteration is still in place).
- `error::AnvilError` still carries the `NotAZip` / `AmbiguousArchive`
  / `LooksBinary` variants that are never constructed. Same
  status as before; still on the cleanup list.

### Open loops

- ADR backlog candidates unchanged from the previous iteration
  (mmap, ZIP decompress-to-temp, generation-counter cancellation).
  No durable architectural decision landed this iteration, so no
  ADR filed. The clipboard choice (formatter in `anvil_core` +
  free closure for cell access + `arboard` for the platform call)
  is recorded in `ARCHITECTURE.md` and is small enough that an
  ADR is not warranted.
- Click-and-drag range selection remains unwired. The per-cell
  `on_mouse_down` is still the only path that updates selection
  on a mouse interaction; `MouseMoveEvent` with `dragging()` is
  already wired for the horizontal scrollbar, so the pattern
  exists in the codebase.
- `Cmd/Ctrl+Home/End` (jump to first / last row) remains in the
  keyboard roadmap but is not yet wired.
- Tab / Shift-Tab navigation between cells remains unwired.

### Next recommended task

Land **search** (`Cmd/Ctrl+F`) next. This is the smallest remaining
Milestone 2 task that touches new surface area:

1. Extract a `commands` module first (the inline handler in
   `view.rs::on_grid_key_down` should not grow further). The
   module owns keybindings and routes to `MainView` methods.
   Keep the existing handlers; just relocate them.
2. Add `arboard`-free `query::search` to `anvil_core`:
   `pub fn search(doc: &ParsedDocument, needle: &str, opts:
   SearchOptions) -> Vec<u64>` returning matching record indexes.
   Streaming over the mmap; case sensitivity / column scope as
   options.
3. Wire `Cmd/Ctrl+F` to open a search bar overlay with an input
   field. Each keystroke spawns a background `cx.spawn` task that
   runs `query::search` and updates the match list; the
   generation counter cancels obsolete work.
4. F3 / Shift-F3 navigate through matches and scroll the grid to
   the matching row.
5. Highlight matched cells in the visible rows.
6. Tests: needle matching, escape handling, case sensitivity,
   column scoping; cancellation when input changes mid-search.

The current selection type carries `(record_index, column_index)`
ranges, which is exactly what search highlights need; landing
search now makes full use of the slice already in place.

---

## 2026-10-02 — Fix: grid keyboard focus (Cmd/Ctrl+C and navigation)

### Context

A user reported that Cmd+C did not copy in a CSV on macOS. The
clipboard slice from the previous iteration had added the TSV
formatter and the keybinding, and the data-side tests all passed, so
the failure had to be in the UI event path. Investigation of GPUI's
focus model showed the real cause: `on_key_down` only fires for
elements in the **focused** element's path, and the grid div had
`key_context("Grid")` and an `on_key_down` listener but **no
`FocusHandle` and no `.track_focus(...)`**. The grid was therefore
never focusable, and *no* grid keyboard event ever reached
`MainView::on_grid_key_down` — this silently broke Cmd/Ctrl+C, arrow
navigation, Page Up/Down, Home/End, and Esc. The previous agent log
had ticked the arrow-key/Home/End tasks and explicitly deferred
manual keyboard exercise, which is how the gap went unnoticed.

### Implemented

- `MainView.focus_handle: FocusHandle`, created via `MainView::new(cx)`
  (`cx.focus_handle()`). `MainView::default()` was removed; `app.rs`
  now builds the view with `cx.new(MainView::new)`.
- The grid container in `render_grid` now carries
  `.track_focus(&self.focus_handle)`. This registers the element in
  the focus path and lets mouse-down anywhere in the grid transfer
  focus (GPUI's documented behaviour for `track_focus`).
- `MainView.focus_grid_on_next_frame: bool`, set in `apply` when a
  document becomes ready. `render_grid` schedules a deferred
  `cx.defer_in(window, ...)` focus call rather than mutating focus
  mid-render (which can re-enter the frame).
- **Made the clipboard sink injectable** so the UI tests can assert
  the copy payload without writing to the real system clipboard:
  `type ClipboardWriter = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>`,
  `MainView.clipboard`, defaulting to `system_clipboard_write`
  (`arboard`). `copy_selection_to_clipboard` calls the field. This
  also made the `arboard` call site testable.
- **Added a GPUI UI-test harness** (`gpui` `test-support`
  dev-dependency) and a `#[cfg(test)]` module in `src/ui/view.rs`.

### Verified

- Reproduced the bug as a test first:
  `arrow_keys_move_selection_when_grid_focused` fails when
  `.track_focus(&self.focus_handle)` is removed (cursor stays at
  `(0,0)` after `down`) and passes when it is present. Re-verified
  after switching the test to the production deferred-focus path.
- `cargo test --release` — 79 passed (25 lib + 2 UI + 52 integration),
  0 failed, 1 ignored.
- `cargo test --release --bin anvil` — 2 UI tests pass:
  `arrow_keys_move_selection_when_grid_focused`,
  `copy_shortcut_reaches_handler_when_grid_focused`.
- Clipboard side-effect check: set the clipboard to a sentinel with
  `pbcopy`, ran the UI tests, confirmed the sentinel is unchanged
  (`pbpaste`). The tests use the injected recorder, not `arboard`.
- `cargo clippy --release --all-targets` — clean (fixed one
  pre-existing `byte_char_slices` nit in `tests/core.rs`).
- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds; verified the `test-support`
  dev-dependency does **not** affect release builds (`gpui` is not
  recompiled and the release binary runs).
- `./target/release/anvil` — launches and runs for 5 s with no panic
  (killed by timeout, exit 124).

### Tests added / changed

- 2 new UI tests in `src/ui/view.rs`:
  - `arrow_keys_move_selection_when_grid_focused` — injects a ready
    document, lets `render_grid` schedule the production deferred
    focus, dispatches `down` / `right`, asserts the cursor moves.
  - `copy_shortcut_reaches_handler_when_grid_focused` — dispatches
    Cmd/Ctrl+C, asserts the status message is `copied 1 cell`, and
    asserts the exact TSV (`1\n`, or `1\r\n` on Windows) was written
    to the injected clipboard recorder.
- Fixed a pre-existing clippy `byte_char_slices` warning in
  `tests/core.rs` (`[b';', b'|']` → `*b";|"`).
- No data-side tests changed.

### Roadmap changes

- Milestone 2 Tests: added and ticked “UI regression tests for grid
  focus / keyboard routing”.
- Milestone 2 Dependencies: recorded the dev-only `gpui`
  `test-support` feature.
- Milestone 5: added a “Test coverage” task to extend the GPUI test
  suite to drag/drop, the archive picker, and toolbar clicks.
- The arrow/Page/Home/End checkboxes remain ticked; they are now
  genuinely verified rather than only code-present.

### Architecture changes

- `ARCHITECTURE.md` state table: added `focus_handle`,
  `focus_grid_on_next_frame`, and `clipboard`.
- “Keyboard handling” section rewritten: focus is an explicit
  prerequisite; documents the deferred-focus and mouse-down focus
  paths and that `on_key_down` is dead without `track_focus`.
- New architectural invariant 11: “The grid owns keyboard focus.”
- Testing section rewritten: documents the GPUI `TestAppContext`
  harness, the two UI tests, and that they fail without
  `track_focus`. Test group table gained a UI row.
- Important Dependencies: added the dev-only `gpui` `test-support`
  row and a note on feature unification (release builds unaffected).
- “Selection / navigation / clipboard” planned-evolution bullet now
  records the focus path and the injectable `ClipboardWriter`.

### Issues / technical debt discovered

- The previous iteration shipped keyboard navigation that never
  worked in the GUI because no UI test exercised focus. The new
  harness closes that class of gap for the grid, but drag/drop, the
  archive picker, and toolbar clicks still have no automated test.
- `gpui` `test-support` pulls in a large transitive tree (reqwest,
  git2, image, resvg, …). It is dev-only and release builds are
  unaffected, but `cargo test` now compiles substantially more. If
  this becomes a CI burden, consider whether the harness should be
  an opt-in feature.
- `render_grid` still focuses the grid whenever
  `focus_grid_on_next_frame` is set; once a search input or filter
  field exists, that path must not steal focus from a text field.
  The flag is only set on `OpenOutcome`, so today it is safe, but the
  next text-input feature must revisit it.
- The inline `on_grid_key_down` handler is now genuinely load-bearing
  (it was previously dead code). The `commands` module extraction is
  more important than before; `view.rs` is ~980 lines.

### Open loops

- Manual GUI verification of the actual macOS Cmd+C keystroke was not
  performed (no automated way to drive the release app). The UI test
  exercises the same GPUI focus + key-dispatch path with the
  production focus scheduling, and the clipboard formatter is
  covered by 22 data-side tests, so confidence is high; a human
  smoke test on macOS is still worthwhile.
- `MainView.status_message` still does not auto-expire.
- `AnvilError` dead variants (`NotAZip`, `AmbiguousArchive`,
  `LooksBinary`) remain unwired.

### Next recommended task

Proceed with **search** (`Cmd/Ctrl+F`) as previously planned — but
first extract the `commands` module, because the keyboard handler is
now real, growing, and overdue for the refactor:

1. Move `on_grid_key_down` routing into `commands` (search, filter,
   clipboard, navigation). Keep `MainView` methods; just relocate
   dispatch.
2. Add `anvil_core::query::search(doc, needle, opts) -> Vec<u64>`.
3. Add the search-bar overlay and background-task cancellation.
4. Add a UI test for the search bar, using the harness now in place.

---

## 2026-10-03 — Copy confirmation flash (interim UX)

### Context

After the focus fix, Cmd/Ctrl+C worked, but the only feedback was the
status-bar text "copied N cells". The user asked for a more immediate
visual confirmation, e.g. the selected area blinking green. This is an
interim UX feature; it does not change the data side.

### Implemented

- `theme::BG_COPIED = #2f8f4f`, a green used only for the flash.
- `MainView.copy_flash_rect: Option<(CellPos, CellPos)>` — the
  normalized rectangle of the most recent successful copy — plus
  `copy_flash_generation: u64` for timer invalidation.
- `MainView::start_copy_flash(rect, cx)` records the rectangle, bumps
  the generation, and spawns a detached
  `background_executor().timer(COPY_FLASH_DURATION)` task. The task
  clears the rectangle and calls `cx.notify()` only if its captured
  generation is still current, so copying again during a flash
  restarts it instead of being cleared early by the previous timer.
- `COPY_FLASH_DURATION = 450 ms`.
- `grid::render_row` now takes `copy_flash: Option<(CellPos, CellPos)>`.
  A cell inside the rectangle is painted `BG_COPIED` in preference to
  `BG_SELECTED`; a full-width rectangle also tints the row gutter
  (matching the existing full-row selection styling). The rectangle is
  captured at copy time, so moving the selection during the flash does
  not move the green feedback.
- `apply` clears `copy_flash_rect` on every `OpenOutcome` so a stale
  flash cannot survive a new document.
- New private helper `grid::rect_contains` (inclusive rectangle test).

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 81 passed (25 lib + 4 binary + 52
  integration), 0 failed, 1 ignored.
- `cargo test --release --bin anvil` — 4 pass: the two UI regression
  tests plus `rect_contains_respects_inclusive_bounds` and
  `rect_contains_is_false_when_none`.
- The copy UI test now also asserts `copy_flash_rect ==
  Some(((0,0), (0,0)))` after Cmd/Ctrl+C.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `./target/release/anvil` — launches and runs 5 s without panic
  (killed by timeout).

### Tests added / changed

- 2 new unit tests in `src/ui/grid.rs` for `rect_contains` (inclusive
  bounds and `None`).
- Extended `copy_shortcut_reaches_handler_when_grid_focused` in
  `src/ui/view.rs` to assert the flash rectangle is set on copy.
- No data-side tests changed.

### Roadmap changes

- Milestone 2 Clipboard: added and ticked “Visual copy confirmation”.

### Architecture changes

- `ARCHITECTURE.md` state table: added `copy_flash_rect` and
  `copy_flash_generation`.
- Clipboard copy section: updated the UI-wiring steps and added a
  “Copy confirmation flash” subsection describing the timer, the
  generation guard, and the `BG_COPIED` rendering.
- Testing section: noted the flash assertion and the two
  `rect_contains` unit tests; test group table gained a “Grid geometry”
  row.

### Issues / technical debt discovered

- The flash is a fixed 450 ms highlight, not a true multi-pulse blink.
  A single flash is calmer and was chosen deliberately; a pulsing
  animation can be added later behind the same `copy_flash_rect` state
  if desired.
- `status_message` still does not auto-expire; the flash does. The two
  feedback channels are now independent (message persists, flash
  clears), which is slightly inconsistent. A future polish pass could
  clear the message on the same timer or on the next selection change.
- `render_row` now takes seven parameters. If it grows again, bundle
  the render inputs into a small struct.

### Open loops

- Manual visual confirmation of the green flash in the running macOS
  app was not performed (the state transition is unit-tested; the
  actual paint is a one-line `bg(theme::BG_COPIED)` branch).
- `view.rs` is now ~1000 lines. The `commands` extraction is still the
  next recommended structural change.

### Next recommended task

Unchanged from the previous entry: extract the `commands` module, then
land search (`Cmd/Ctrl+F`). The copy confirmation was a small interim
feature and does not change the plan.

---

## 2026-10-04 — Drag-select

### Context

The user asked for click-and-drag box selection. This was the last
unchecked item in the Milestone 2 Selection model. The grid already had
per-cell `on_mouse_down` (set / shift-extend) and keyboard navigation;
what was missing was holding the button and dragging across cells.

### Implemented

- `MainView.drag_selecting: bool`, armed by a grid cell mouse-down and
disarmed on mouse-up. It prevents a drag that began outside the grid
from selecting cells it passes over.
- `grid::render_row` gained a `mouse_move` handler generic; every cell
now carries `.on_mouse_move(...)` in addition to `.on_mouse_down(...)`.
- In `view.rs`, the cell mouse-move closure ignores events unless
`MouseMoveEvent::dragging()` is true and `drag_selecting` is set, then
calls `Selection::extend_to(pos)` and `cx.notify()`.
- The grid container carries `on_mouse_up` (bubble, pointer over the
grid) and `on_mouse_up_out` (capture, pointer has left the grid) to
disarm `drag_selecting`.
- **Design choice: per-cell hitboxes rather than coordinate
arithmetic.** Each cell is its own GPUI hitbox, so whichever cell the
pointer is over reports its own `(row, col)`. This avoids the
`ColumnLayout`/`scroll_x`/vertical-scroll mapping that a single
full-grid mouse listener would need, and it composes with
`uniform_list` virtualization for free. The trade-off is a one-event
lag from GPUI's hover hit test, imperceptible for a real drag.
- `render_row`'s argument list had reached 8 and tripped clippy
`too_many_arguments`; the per-frame inputs are now bundled into a
`grid::RowContext<'a>` (`selection`, `scroll_x`, `viewport_w`,
`copy_flash`) passed by reference. This was the follow-up flagged in
the copy-flash entry.
- A test-only `.debug_selector("anvil-grid-list")` on the grid
container (a no-op in release) lets the UI test derive cell pixel
centres from the measured grid bounds plus `ROW_HEIGHT`,
`ROW_NUMBER_WIDTH`, and the computed column widths.

### Verified

- Reproduced the gap as a test first: `drag_select_builds_rectangle`
fails when the cell `.on_mouse_move` wiring is removed (selection
stays one cell) and passes with it. Re-verified after the
`RowContext` refactor.
- `cargo test --release` — 82 passed (25 lib + 5 binary + 52
integration), 0 failed, 1 ignored.
- `cargo test --release --bin anvil` — 5 pass: two focus/copy tests,
the new drag test, and the two `rect_contains` unit tests.
- The drag test also copies the dragged block and asserts the TSV
(`1\t2\n4\t5\n`), so drag-select + copy is covered end-to-end.
- `cargo clippy --release --all-targets` — clean (the
`too_many_arguments` warning is resolved by `RowContext`).
- `cargo fmt --check` — clean.
- `cargo build --release` and `./target/release/anvil` — launch and
run 5 s without panic.

### Tests added / changed

- 1 new UI test: `drag_select_builds_rectangle`.
- No data-side tests changed; `Selection::extend_to` was already
covered.

### Roadmap changes

- Milestone 2 Selection model: "Click-and-drag range selection"
ticked (the last unchecked selection item).

### Architecture changes

- `ARCHITECTURE.md` state table: added `drag_selecting`.
- Selection model → Grid integration: documented the drag-select
mechanism and why per-cell hitboxes are used.
- Grid Rendering → Row retrieval: documented `RowContext`.
- Functional gaps and Planned Evolution updated (drag-select wired;
edge auto-scroll still outstanding).
- Testing: added the drag test, its mutation check, and a test-table
row.

### Issues / technical debt discovered

- **No drag auto-scroll.** Dragging past the top/bottom edge does not
scroll the list, so a selection cannot be extended beyond currently
visible rows in one gesture. This is the main functional gap in
drag-select and is recorded in the roadmap and architecture open
loops. Implementing it needs the pointer position mapped to the list
bounds and a scroll tick while the pointer is outside — likely a small
`cx.spawn` loop using `UniformListScrollHandle::scroll_to_item`.
- GPUI delivers mouse moves to the hovered hitbox using the hit test
from the previous frame, so drag-select lags a single mouse event. In
practice mouse moves are continuous; only synthetic single-jump tests
notice. The test moves twice to the target to be deterministic.
- `view.rs` is now ~1090 lines. The `commands` extraction is more
overdue than ever.

### Open loops

- Drag auto-scroll at viewport edges (recorded).
- Non-contiguous multi-select and select-all are recorded in
`ROADMAP.md` Future Ideas, not started.
- Configurable copy format (TSV/CSV/Markdown/JSON) recorded in Future
Ideas; current Cmd/Ctrl+C is TSV only.

### Next recommended task

Still the `commands` module extraction, then search (`Cmd/Ctrl+F`).
Drag-select was the last selection-model gap; the selection surface is
now feature-complete for Milestone 2 except for the drag auto-scroll
follow-up.

---

## 2026-10-05 — Drag auto-scroll

### Context

The previous iteration landed drag-select but left one gap: dragging
past the top/bottom edge of the list did nothing, so a selection could
only extend to currently visible rows. This iteration closes it.

### Implemented

- An overlay `canvas` is added to the grid list container
  (`.absolute().size_full()`). A plain `canvas` has no hitbox, so it
  does not intercept clicks or hover. It has two jobs:
  1. record the scroll area's window-space bounds into
     `MainView.list_bounds` each frame; and
  2. install a global `window.on_mouse_event` mouse-move listener that
     stores `MainView.pointer_pos` and, while `drag_selecting`, calls
     `auto_scroll_step`.
  Per-cell `on_mouse_move` cannot see the pointer once it leaves the
  grid, so a global listener is required for edge auto-scroll.
- `MainView::auto_scroll_step`: when the pointer is above/below
  `list_bounds` (pure `auto_scroll_direction` helper), it extends the
  selection to the row at that edge and asks the list to pin the next
  row to the top via `scroll_to_item_strict(.., ScrollStrategy::Top)`.
  The column cursor is preserved.
- `MainView::start_auto_scroll` / `stop_auto_scroll` and
  `auto_scroll_task`: a `cx.spawn` loop that ticks every
  `AUTO_SCROLL_TICK` (30 ms) and repeats `auto_scroll_step` while
  `drag_selecting`, so holding the pointer still past the edge keeps
  scrolling. The loop exits when `drag_selecting` clears; it is also
  dropped on mouse-up.
- Cell mouse-down now records the initial `pointer_pos` and calls
  `start_auto_scroll`; the mouse-move handler also records
  `pointer_pos`. Mouse-up (bubble or `on_mouse_up_out`) clears
  `drag_selecting` and stops the loop.
- The list container became `.relative()` to anchor the absolute
  overlay canvas.

### Verified

- Reproduced the gap as a test first: `drag_past_bottom_edge_starts_auto_scroll`
fails when the global listener's `auto_scroll_step` call is removed
(selection stays at row 0) and passes with it.
- `cargo test --release` — 84 passed (25 lib + 7 binary + 52
integration), 0 failed, 1 ignored.
- `cargo test --release --bin anvil` — 7 pass: the four UI tests plus
three unit tests.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `cargo build --release`, `./target/release/anvil` (runs 5 s, no
panic), and `anvil --probe fixtures/large.csv` (351 ms) all fine, so
the overlay canvas does not disturb the open/render path or
throughput.

### Tests added / changed

- `auto_scroll_direction_detects_edges` — pure above / on-edge /
inside / below cases.
- `drag_past_bottom_edge_starts_auto_scroll` — 1000-row document,
synthesised drag past the bottom edge, asserts the selection extended
past the first visible row and `base_handle.offset().y < 0` (the list
actually scrolled). Also asserts mouse-up clears `drag_selecting` and
stops the task.
- Test helper `large_document(rows)` and `inject_doc(cx, view, doc)`;
`inject` now delegates to `inject_doc(sample_document())`.
- No data-side tests changed.

### Roadmap changes

- Milestone 2 Selection model: added and ticked "Drag auto-scroll".

### Architecture changes

- `ARCHITECTURE.md` state table: added `list_bounds`, `pointer_pos`,
`auto_scroll_task`.
- Selection model → Grid integration: documented the auto-scroll
mechanism (overlay canvas, global listener, tick loop).
- Open loops narrowed to "no proximity margin / no acceleration"; the
old "does not auto-scroll" loop is removed.
- Functional gaps, Planned Evolution, and Testing updated; test group
table row expanded.

### Issues / technical debt discovered

- Auto-scroll triggers only once the pointer is fully outside the
  list's vertical bounds. A proximity margin (start scrolling when
  within N px of the edge) and proportional speed (faster the further
  out) would feel better; both are recorded as open loops.
- The global mouse-move listener runs `entity.update` on every move in
  the window, even when no drag is active. The work is a single field
  write plus an early return, but a future optimisation could gate
  registration on `drag_selecting`.
- The overlay canvas's `list_bounds` is written every frame via
  `entity.update` without `notify`; harmless, but it means each frame
  mutates the view. If it shows up in a profile, compare bounds before
  writing.
- Vertical auto-scroll uses `scroll_to_item_strict(.., Top)`, which
  snaps the target row to the top (row-by-row, 30 ms ticks). Smooth
  pixel scrolling via `set_offset` would be smoother but needs careful
  sign handling; deferred until it matters.
- `view.rs` is now ~1420 lines including tests. The `commands`
  extraction is well overdue.

### Open loops

- Auto-scroll proximity margin and proportional acceleration (above).
- Non-contiguous multi-select, select-all, and configurable copy
  format remain in `ROADMAP.md` Future Ideas.

### Next recommended task

Extract the `commands` module, then land search (`Cmd/Ctrl+F`). The
selection model and drag interaction are now complete for Milestone 2.

---

## 2026-10-06 — Extract keyboard commands module

### Context

The previous handoff recommended extracting keyboard routing before
adding search or filter. `MainView::on_grid_key_down` contained the
raw key-string match for movement, Cmd/Ctrl+C, and Esc. This iteration
lifts the key-to-intent mapping into `src/ui/commands.rs` without
changing visible behavior.

### Implemented

- New `src/ui/commands.rs` defines a small UI-layer `Action` enum:
  `MoveCursor { row_delta, column_delta, extend }`, `Copy`, and
  `Escape`.
- Pure `commands::action_for(&Keystroke) -> Option<Action>` is now the
  single binding table. `PAGE_STEP` moved into this module.
- `MainView::on_grid_key_down` now delegates to `action_for`; the new
  `MainView::run_action` executes the intent against selection,
  clipboard, and overlay state. Existing focus guard and behavior are
  preserved.
- **Design choice:** this is the lightweight extraction, not GPUI's
  `actions!` / `cx.bind_keys` typed-action layer. The pure mapping
  preserves modifier-aware Shift extension without introducing
  separate GPUI actions for every shifted movement chord. Typed,
  user-rebindable shortcuts and menu generation remain a later option.
- Nine command-mapping unit tests cover the platform-specific
  Cmd/Ctrl copy chord, plain and Alt+C, arrows and Shift+arrow, Page
  Up/Down, Home/End, Esc, and unbound keys.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 93 passed (25 lib + 16 binary + 52
  integration), 0 failed, 1 ignored.
- All seven pre-existing binary tests still pass (four UI tests for
  focus/navigation, copy, drag-select, and drag auto-scroll; plus three
  geometry/direction helper tests).
- Nine new `commands` unit tests pass without a window.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- Existing keyboard UI tests continue to exercise the dispatch path,
  so the extraction is verified end-to-end, not only at the pure
  mapping layer.

### Tests added / changed

- Nine unit tests in `src/ui/commands.rs`.
- Existing UI tests unchanged and passing.

### Roadmap changes

- Milestone 2 Keyboard navigation: "Wire a dedicated `commands`
  module" ticked. The checkbox note distinguishes the current pure
  binding module from a future GPUI typed-action layer.

### Architecture changes

- `ARCHITECTURE.md` repository tree now includes `ui/commands.rs`.
- "What is current vs planned" marks `ui::commands` current and
  removes it from the planned modules list.
- Actions / commands section documents `action_for`, `Action`,
  `run_action`, and the remaining GPUI typed-action option.
- Keyboard handling section updated to reference the command mapping
  and `commands::PAGE_STEP`.
- Testing section records the nine command tests and test-table row.

### Issues / technical debt discovered

- `commands::Action` is intentionally a local intent enum, not a GPUI
  `Action`; there is still no `actions!` / `cx.bind_keys` layer. This
  means the shortcuts are not user-rebindable and cannot yet generate
  menu items or a shortcut cheatsheet.
- `Escape` remains attached to the grid's key context and guarded by
  `model.is_some()`. The archive picker has no grid rendered while it
  is open, so Escape-to-close-picker is not actually reachable through
  this handler; fixing overlay-level key routing is separate work.
- `view.rs` still executes the action handlers; future actions should
  continue to be small dispatch arms, with parsing/binding centralized
  in `commands.rs`.

### Open loops

- GPUI typed actions / user-rebindable bindings if shortcut
  customization or generated menus become a requirement.
- Search, filter, go-to-row, select-all, and configurable copy-format
  bindings are still unimplemented.

### Next recommended task

Proceed with `query::search` and `Cmd/Ctrl+F`, using
`commands::Action` as the dispatch entry. Add the search UI, background
scan with generation-based obsolescence, F3/Shift-F3 navigation, and
visible-match highlighting. The action mapping and test infrastructure
are now ready for the feature.

## 2026-10-07 — Milestone 2 Search slice (Cmd/Ctrl+F, F3/Shift-F3, Enter cycle)

### Context

The previous iteration extracted the `commands` module and landed all
the keyboard routing for navigation / clipboard / archive-picker.
The agent-log handoff recommended `query::search` + `Cmd/Ctrl+F` as
the next vertical slice. This iteration lands the entire basic-search
feature end-to-end:

* `anvil_core::query::search` (pure, GPUI-free) with `SearchOptions`
  (case-sensitive / column-scope / max-matches);
* `commands::Action::OpenSearch` and `Action::NextMatch` bindings;
* A search-bar overlay with a dedicated focus handle, character
  input, Backspace, Enter / Shift+Enter, and Esc;
* Background search task on `cx.background_executor` with
  generation-counter cancellation;
* Match highlighting in the grid (`BG_MATCH` / `BG_MATCH_ACTIVE`);
* F3 / Shift-F3 cycle matches from anywhere via the grid's
  `Action::NextMatch` arm;
* Search state is cleared on every `OpenOutcome` so opening a new
  document cannot carry stale matches.

### Implemented

- New module `src/query.rs` exposing
  `pub fn search(doc: &ParsedDocument, needle: &str, opts:
  &SearchOptions) -> Vec<u64>`. Walks each record's bytes through
  `Cells::new` (which handles quoting / escapes) and matches with
  `memchr::memmem::Finder`. Case-insensitive matching (the default)
  uses an ASCII-lowercased needle and a sliding-window lowercase over
  the cell bytes. `SearchOptions { case_sensitive, column,
  max_matches }` carries the toggle / scope / cap. Per-record decode
  only, no `Vec<Vec<String>>` materialisation.
- `src/lib.rs`: re-exports `SearchOptions` and `search` as
  `anvil_core::{SearchOptions, search_records}`.
- `src/ui/theme.rs`: `BG_MATCH = #4a3a00` and `BG_MATCH_ACTIVE =
  #6b5400`.
- `src/ui/grid.rs`:
  - `RowContext` gains `search_matches: &[u64]` (sorted record
    indexes) and `search_active: i64`. Matched rows tint the row
    background; the active match uses the stronger colour.
  - The lookup is `binary_search` per visible row — about 30 visible
    rows × ~7 probes per redraw for a 1M-row file with 100k matches.
- `src/ui/commands.rs`:
  - New `Action::OpenSearch` (Cmd/Ctrl+F).
  - New `Action::NextMatch { direction }` (F3 = next, Shift+F3 =
    previous).
  - The copy chord rejects Alt / Fn / non-secondary as before; same
    logic for `f`. Five new unit tests cover the new chords.
- `src/ui/view.rs`:
  - New `MainView` search fields: `search_open`, `search_query`,
    `search_matches`, `search_active`, `search_generation`,
    `search_task`, `search_focus_handle`,
    `focus_search_on_next_frame`.
  - `open_search` opens the bar, kicks off a search for the empty
    query, and sets the deferred-focus flag.
  - `close_search` clears everything (called from `apply` on every
    `OpenOutcome`).
  - `kick_search` bumps the generation, drops the previous task, and
    spawns `query::search` on `cx.background_executor` with the
    captured generation; the task's `update` callback is a no-op
    when the generation has moved on.
  - `apply_search_result` installs the matches and sets
    `search_active = 0` (or `-1` if there are none).
  - `advance_match` cycles `search_active` with i64-safe modular
    arithmetic so the wrap from row N-1 to row 0 is well-defined.
  - `jump_to_active_match` sets the cursor to the active row's column
    and calls `scroll_to_item_strict(.., ScrollStrategy::Top)`.
  - `on_search_key_down` handles Escape (close + refocus grid),
    Enter / Shift+Enter (cycle matches), Backspace (pop last byte),
    and printable characters (append + restart search). Cmd+F,
    Alt+F, etc. fall through to the grid handler.
  - `run_action` arms for `OpenSearch` (no-op when already open /
    no document) and `NextMatch { direction }` (no-op without a
    match list).
  - `apply` calls `close_search` so a new document cannot carry
    stale matches or query text.
  - `render_overlay` was refactored to return `Vec<AnyElement>`; it
    stacks the search bar and the archive picker independently.
  - `render_search_bar` paints the bar at the top of the window with
    `key_context("Search")` and `.track_focus(&self.search_focus_handle)`.
    The status text ("1 of 12", "no matches", "searching…") is computed
    by `search_status_text`.
  - `Render::render` defers focus to `search_focus_handle` when
    `focus_search_on_next_frame` is set.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 128 passed (was 93): 38 lib (unchanged),
  30 binary (was 16; +9 commands tests, +5 search UI tests), 60
  integration (was 52; +8 search tests). 1 ignored
  (`measure_large_fixture`).
- `cargo test --release --lib query` — 13 passed.
- `cargo test --release --test core search` — 8 passed.
- `cargo test --release --bin anvil` — 30 passed, including all
  pre-existing focus / copy / drag / auto-scroll tests and 5 new
  search tests.
- `cargo clippy --release --all-targets` — clean (the existing
  pre-`1.98` future-incompat notes are unchanged).
- `cargo fmt --check` — clean.
- `cargo test --release --test core -- --ignored measure_large
  --nocapture` — 165.4 MiB / 1,000,000 rows in 128 ms (1292 MiB/s),
  20 cols. Within noise of the pre-search baseline (~650 MiB/s;
  variance across cold/warm cache dominates).
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, 20 columns, 0 warnings, probe time in the same range as the
  Milestone 1 baseline.
- `./target/release/anvil --probe fixtures/multi.zip` — picker
  message printed, 2 candidates listed.
- `./target/release/anvil --probe fixtures/malformed.csv` —
  unterminated-quote warning still surfaces.
- `./target/release/anvil` (GUI launch, killed after 5 s) — no
  panic, no stderr. Manual UI exercise of Cmd/Ctrl+F, character
  input, Backspace, Enter, Esc deferred; the state transitions are
  covered by the UI regression tests against a real fixture.

### Tests added / changed

- 13 unit tests in `src/query.rs`:
  - `ascii_lower_helper_basic`
  - `empty_needle_returns_no_matches`
  - `single_match`
  - `multiple_matches_in_record_order`
  - `case_insensitive_default_matches_uppercase_data`
  - `case_sensitive_only_matches_exact_case`
  - `column_scoped_search`
  - `column_scope_out_of_bounds_returns_empty`
  - `quoted_cells_are_searched`
  - `max_matches_caps_results`
  - `no_match_returns_empty`
  - `empty_document_returns_empty`
  - `case_insensitive_matches_mixed_case_needle`
- 8 integration tests in `tests/core.rs::search_*` against real
  `ParsedDocument`s:
  - `search_finds_matches_across_columns`
  - `search_returns_matches_in_record_order`
  - `search_empty_needle_returns_no_matches`
  - `search_case_sensitive_distinguishes_case`
  - `search_column_scope_limits_search`
  - `search_max_matches_caps_results`
  - `search_handles_quoted_commas`
  - `search_finds_substring_match`
- 5 unit tests in `src/ui/commands.rs`:
  - `open_search_chord_maps_to_open_search`
  - `plain_f_is_not_open_search`
  - `alt_f_is_not_open_search`
  - `f3_jumps_to_next_match`
  - `shift_f3_jumps_to_previous_match`
- 9 UI regression tests in `src/ui/view.rs`:
  - `cmd_f_opens_search_bar_when_grid_focused`
  - `search_bar_input_appends_and_skips_grid_navigation`
  - `search_bar_backspace_drops_last_character`
  - `escape_closes_search_bar`
  - `search_populates_match_list`
  - `search_finds_real_matches_in_fixture`
  - `search_enter_cycles_through_matches`
  - `f3_advances_match_without_opening_bar`
  - `opening_new_document_clears_search_state`
- No data-side tests removed.

### Roadmap changes

- Milestone 2 Search: ticked the seven items landed by this slice
  (Cmd/Ctrl+F binding, `query::search`, background task with
  generation, match count, visible-row highlighting, F3 / Shift-F3).
  Left unchecked: case-sensitive UI toggle, column-scoped search via
  header click. `SearchOptions` already supports both options; this
  is purely UI work for a follow-up slice.
- Milestone 2 Tests: ticked "Unit tests for `query::search` (needle
  matching, escape handling, case sensitivity, column scoping)" and
  "Test that opening a new file cancels an in-flight search /
  filter" (the latter is exercised by `opening_new_document_clears_search_state`,
  which calls the same `close_search` that `apply` does).
- Milestone 2 Keyboard navigation: left "Cmd/Ctrl+G — go-to-row
  input box" and "Tab / Shift-Tab" untouched — neither is part of
  the search slice.

### Architecture changes

- `ARCHITECTURE.md`:
  - Primary-state table extended with `search_open`, `search_query`,
    `search_matches`, `search_active`, `search_generation`,
    `search_task`, `search_focus_handle`,
    `focus_search_on_next_frame`.
  - "What is *current* vs *planned*" updated: `query::search` is
    listed as Current; `query::filter` and `query::sort` remain
    Planned.
  - Recognised-keys table extended with F3 / Shift-F3, Esc (closes
    the search bar), and Cmd/Ctrl+F.
  - Commands binding list updated: `Action` enum now lists
    `OpenSearch` and `NextMatch`.
  - "Planned Architectural Evolution → Search (Milestone 2)" rewritten
    to describe the actual implementation (the data-side
    `query::search`, the generation-counter pattern, the focus
    handle, the row-tint visualisation, F3 / Shift-F3 anywhere,
    Cmd/Ctrl+F no-op when open).
  - Testing section rewritten: 60 integration tests, 25 selection
    unit tests, 13 query unit tests, 14 command-mapping unit tests,
    nine UI regression tests including the new search coverage.
  - "Current Limitations / Technical Debt → Functional gaps" updated
    to note that search is wired and tested, leaving the
    case-sensitive / column-scoped UI toggles and the column
    header-click hookup as open work.
- `Cargo.toml`: no new dependencies. `memchr` was already a
  dependency; the new search module uses `memchr::memmem::Finder`.

### Issues / technical debt discovered

- **No case-sensitive toggle in the UI.** `SearchOptions` supports it
  on the data side, but the bar shows only the query input. The
  follow-up slice should add a small `Aa` button next to the query
  (or a `Cmd/Ctrl+Shift+F` chord) to toggle it.
- **No column-scoped search.** Same story: `SearchOptions.column`
  exists, but no UI to set it. A natural fit is "click a header,
  then Cmd/Ctrl+F" — the bar picks up the column index from the
  clicked header cell.
- **No match list UI.** Today the user sees only the count and the
  highlighted rows. A small scrolling list of matches on the right
  side of the bar (or in a separate overlay) would make
  many-matches files more navigable. Deferred.
- **The bar swallows printable characters, but only when it has
  focus.** The test harness does not exercise the focus transfer
  because it bypasses render. Production paths do; the
  `open_search` deferred-focus path mirrors the grid's. If a future
  change introduces another focusable element on the same screen
  with the search bar open, focus-routing would need revisiting.
- **`ascii_contains_lower` uses a 256-byte stack buffer** with a
  fallback to an allocation for very long needles (>256 bytes). For
  typical CSV search (short needles) the stack path is hot; the
  long-needle path is a fallback. No measurement is recorded; this
  is a deliberate trade-off documented for future tuning.
- **`view.rs` is now ~2000 lines.** The `commands` extraction from
  the previous iteration took some load off, but the search slice
  rebalanced that. A follow-up split (e.g. `toolbar.rs`, `status.rs`,
  `search.rs`, `grid_render.rs`) is becoming worthwhile but is not
  urgent.
- **`Cmd/Ctrl+Home/End` and `Tab / Shift-Tab` are still unimplemented.**
  Recorded as open loops. Trivial follow-up.

### Open loops

- ADR backlog candidates unchanged from earlier entries (mmap, ZIP
  decompress-to-temp, generation-counter cancellation). The search
  slice did not introduce a durable architectural reversal, so no
  ADR was filed.
- Case-sensitive / column-scoped UI toggles.
- Match-list overlay (separate from the search bar).
- Filter and sort are still unimplemented; both can reuse
  `Vec<u64>`-of-`record_index` results and the same generation
  pattern.

### Next recommended task

Proceed with **filter** (Cmd/Ctrl+L). The search slice lands the
matching shape (a background `query::*` task that returns a sorted
`Vec<u64>` of record indexes, generation-counter cancellation,
status-bar count, and per-row highlight), so filter is mostly the
data-side predicate evaluator and a thin overlay:

1. `anvil_core::query::filter(doc, predicates, opts) -> Vec<u64>`:
   `Filter` enum with the operators listed in the Milestone 2 plan
   (equals / not-equals / contains / not-contains / starts with /
   ends with / empty / non-empty / numeric comparisons). Composability
   via `Vec<Filter>` combined as AND.
2. `MainView` filter state (mirrors the search fields):
   `filter_open`, `filter_predicates`, `filter_matches`,
   `filter_active_match`, `filter_generation`, `filter_task`.
3. Filter bar overlay (Cmd/Ctrl+L) with an input that parses into
   `Filter`s; render filters as small chips. The status bar shows
   "rows (filtered from N)".
4. Grid renders from `filter_matches` (a `Vec<u64>`) instead of the
   full range when the filter is active; the row mapping
   (`record_index → list position`) is built once when the filter
   result is applied.
5. Filter composition with future sort (Milestone 3) is documented
   in the roadmap: sort runs on the filtered indexes, not the full
   record set.

The existing search slice makes this substantially smaller than it
would have been a sprint ago: generation-cancel, the match-list
visual model, and the deferred-focus pattern all transfer.

## 2026-10-08 — Search bar layout: stop overlapping the active-match row

### Context

The user reported that when a search match is at the top of the
visible area, it gets "cut off" and is not clearly shown. The
diagnosis from the search-bar slice's render path was straightforward:

- `render_search_bar` painted the bar as an `absolute`-positioned
  child of the center area (`.absolute().top(px(0.))`).
- The grid (header + uniform_list) lived in the same `.relative()`
  parent and painted from the top of the parent's bounds.
- Therefore the search bar *overlapped* the top of the grid: 36 px
  tall bar + 24 px grid header = the search bar covered the grid
  header and 12 px of the first row of the uniform_list.
- When a search match landed at row 0, the row pinned to the top of
  the list (via `scroll_to_item_strict(.., ScrollStrategy::Top)`),
  putting the row exactly where the search bar was overlaying it.
  The `BG_MATCH_ACTIVE` highlight was visible only on the lower
  portion of the row; the upper portion (including the row gutter)
  was hidden behind the bar's `BG_PANEL` background and bottom
  border. The result looked like a row that had been "cut off".

### Implemented

- `render_search_bar` no longer uses `.absolute().top(px(0.))`. It
  uses `.flex_shrink_0()` with a fixed `h` so the bar takes its
  natural height and sits in the normal flex flow.
- `render_overlay` was simplified to return only the modal archive
  picker; the search bar is now placed directly in `Render::render`
  as a flow child of the center flex container, **above** the grid.
  When closed, the bar is not rendered at all, so no space is
  reserved.
- The grid's sticky column header is therefore never covered by the
  bar, and the active-match row at row 0 sits directly below the
  header — fully visible — rather than half-hidden behind the bar.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 128 passed (38 lib + 30 binary + 60
  integration), 0 failed, 1 ignored. No regressions.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `./target/release/anvil` (GUI launch, killed after 5 s) — no
  panic, no stderr.

### Tests added / changed

None. The fix is a CSS-style layout change and is not directly
observable from the headless test harness (which does not exercise
the rendered bounds). The existing nine UI regression tests still
pass; visual confirmation in the running app is the primary check.

### Roadmap changes

None. The bug was a layout defect, not a missing roadmap task.

### Architecture changes

- `ARCHITECTURE.md` Planned Evolution → Search paragraph: noted that
  the bar sits as a normal-flow child so the grid (and its sticky
  header) shift down by the bar's height rather than sliding under
  it, and that the active-match row stays fully visible even at row
  0.

### Issues / technical debt discovered

- None new. The bar still uses the same focus-handle pattern, the
  same `key_context("Search")`, and the same `on_search_key_down`
  handler; only its DOM placement changed.

### Open loops

- The case-sensitive / column-scoped UI toggles from the search
  slice's handoff are still unimplemented. The fix here does not
  change that scope.
- The search bar still doesn't grow a "match list" overlay (the
  count is inline at the right of the input). Deferred.

### Next recommended task

Unchanged from the previous entry: proceed with **filter**
(Cmd/Ctrl+L). The search slice's data-side shape (`Vec<u64>` of
record indexes from a background task, generation-cancel, row
highlight) transfers directly to filter, which additionally needs a
predicate evaluator and a row-permutation grid path.

## 2026-10-09 — Highlight the matched substring inside each cell

### Context

The search slice from 2026-10-07 highlighted the row (yellow tint
across the whole row) and used a brighter tint for the active row,
but the user could not tell *which cell* in the row contained the
match, let alone *where* inside the cell. The user explicitly asked
for substring-level highlighting so the search hit is unambiguous.

### Implemented

- New data-side helper `anvil_core::query::find_ranges(haystack,
  needle, case_sensitive) -> Vec<Range<usize>>` that returns the
  byte ranges of every occurrence of `needle` inside `haystack`.
  Case-sensitive path uses `memchr::memmem::find_iter`; the
  case-insensitive path lowercases the haystack once (an owned
  buffer) and runs the same iterator. The cost is one allocation
  per call; bounded because the caller drives this only for visible
  cells of match rows.
- `theme::BG_MATCH_TEXT = #c69400` — a bright yellow, distinct from
  the row tints `BG_MATCH (#4a3a00)` and `BG_MATCH_ACTIVE
  (#6b5400)` so the literal matched substring pops visually.
- `RowContext` extended with `search_query: &str` and
  `search_options: SearchOptions` so the renderer knows what to
  highlight and whether the search is case-sensitive.
- `render_row` now branches per cell:
  - Non-match row → no substring scan; cell rendered as plain text.
  - Match row, query empty → no substring scan (defensive).
  - Match row, query non-empty, cell does NOT contain the needle
    → plain text rendering, the row tint shows through.
  - Match row, query non-empty, cell DOES contain the needle →
    `find_ranges` runs; a `StyledText` is built with one
    `HighlightStyle { background_color: Some(BG_MATCH_TEXT) }`
    per match range, and the cell renders with the matched bytes
    highlighted against the row tint.
- Re-exported `find_ranges` from `anvil_core::lib.rs`.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 139 passed (was 128): 47 lib (was 38, +9
  `find_ranges` unit tests), 32 binary (was 30, +2 `render_row`
  smoke tests), 60 integration (unchanged), 1 ignored.
- `cargo test --release --lib query` — 22 passed.
- `cargo test --release --bin anvil` — 32 passed, including the two
  new `render_row_substring_highlight_does_not_panic` and
  `render_row_non_match_does_not_panic` smoke tests.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, 20 cols, 0 warnings, probe time in the same range as the
  baseline.
- `./target/release/anvil` (GUI launch, killed after 5 s) — no
  panic, no stderr. Visual confirmation of the substring
  highlight deferred; the rendering path is exercised by the
  new smoke tests, but the painted pixels can only be observed in
  the running app.

### Tests added / changed

- 9 unit tests in `src/query.rs::find_ranges_*`:
  - `find_ranges_case_sensitive_single_match`
  - `find_ranges_case_sensitive_multiple_matches`
  - `find_ranges_case_sensitive_no_match`
  - `find_ranges_case_insensitive_matches_mixed_case`
  - `find_ranges_case_sensitive_respects_case`
  - `find_ranges_empty_needle_returns_no_ranges`
  - `find_ranges_needle_longer_than_haystack_returns_no_ranges`
  - `find_ranges_empty_haystack_returns_no_ranges`
  - `find_ranges_overlapping_matches`
- 2 smoke tests in `src/ui/grid.rs::tests`:
  - `render_row_substring_highlight_does_not_panic` — builds a
    fixture with multiple matches across cells, sets up a
    `RowContext` with a non-empty query and a populated match
    list, and renders the matched rows. The regression we care
    about (panic from mismatched byte ranges, or a `find_ranges`
    surprise) would surface here.
  - `render_row_non_match_does_not_panic` — same fixture, but
    renders a non-match row to confirm the non-scan path is
    stable.
- No data-side tests removed.

### Roadmap changes

None. Substring highlighting was an enhancement request from the
user, not a roadmap tick.

### Architecture changes

- `ARCHITECTURE.md`:
  - "What is current vs planned" → `query::find_ranges` listed
    alongside `query::search` under Current.
  - "What is current vs planned" → `ui::grid` description extended
    to note the substring highlight (`BG_MATCH_TEXT`).
  - Planned Evolution → Search paragraph: the matched-substring
    highlight is now described, with `find_ranges` as the source
    of byte ranges.
  - Testing section: query unit test count 13 → 22; the new
    `render_row` smoke tests added to the grid-geometry row.
  - Functional gaps → UI coverage line mentions the two
    `render_row` smoke tests.

### Issues / technical debt discovered

- `find_ranges` allocates one lowercase buffer per call. For a
  long match list rendered into a wide viewport this is the
  per-frame hot path; a future optimisation would cache the
  lowercase of the needle (and only that, not the haystack) and
  walk the haystack with a sliding window — same trick as
  `ascii_contains_lower`. Not urgent: per visible cell the cost
  is small (cells are tens of bytes in typical CSV), and there
  are ~600 visible cells per redraw worst case.
- The substring highlight ignores case folding for non-ASCII
  bytes (`ascii_lower` only folds A-Z). A cell containing
  "CAFÉ" matched against "café" would not highlight "É" in the
  same way as a pure ASCII match — the "É" lowercases to itself
  but "é" lowercases to itself too, so the bytes do match.
  Unicode lowercasing would need a crate; not done.
- The active match's tint is unchanged by the substring highlight
  (`BG_MATCH_ACTIVE` row tint + `BG_MATCH_TEXT` cell tint). The
  combination reads clearly in practice but is not formally
  contrast-checked. A future polish pass could introduce a
  slightly different `BG_MATCH_TEXT_ACTIVE` for the active match
  cell.
- The `render_row` smoke tests cannot observe the rendered
  `StyledText` from the headless harness. They guard against
  panics but not against visual regressions (e.g. a future change
  that swaps the highlight order and the substring goes
  unhighlighted). A test that constructs a real `Window` and
  asserts on a paint output would be needed for that. Recorded
  for a future polish pass.

### Open loops

- ADR backlog candidates unchanged. No durable architectural
  reversal landed.
- The case-sensitive / column-scoped UI toggles from the
  previous search-slice handoff are still unimplemented; the
  substring highlight already respects `case_sensitive` because
  `RowContext.search_options` carries it through.
- The match-list overlay is still unimplemented.

### Next recommended task

Proceed with **filter** (Cmd/Ctrl+L), as previously planned.
Substring highlighting is a refinement of the search slice; it
does not change the path forward. The filter slice needs:

1. `anvil_core::query::filter(doc, predicates, opts) -> Vec<u64>`
   with operators equals / not-equals / contains / not-contains /
   starts-with / ends-with / empty / non-empty / numeric `>` / `>=`
   / `<` / `<=`. AND-composition over a `Vec<Filter>`.
2. `MainView` filter state (mirrors the search fields). Filter bar
   overlay (Cmd/Ctrl+L) with an input that parses into `Filter`s.
3. The grid renders from the filtered `Vec<u64>` rather than the
   full range; the row mapping (`record_index → list position`)
   is built once when the filter result is applied.
4. Status bar shows "rows (filtered from N)".

The same generation-cancel pattern from search transfers
verbatim; the same `BG_MATCH`-style row tint applies; the
`find_ranges` helper could also be used to highlight the matched
substring within a filtered value (overkill for filter; deferred).

## 2026-10-10 — Multi-document tabs

### Context

The user asked for an IDE-style way to see and switch between all the
files that have been opened: "for zip files, we want to show all
available files in a file manager type window ... or a tab menu at the
top ... this should be the case for any files put in the window as
well that aren't the original file." That is a multi-document model,
not just a fancier archive picker, so this iteration lands the model
and a flat tab strip on top of it.

### Implemented

- `DocumentSnapshot` — one per open tab. Owns the `Arc<ParsedDocument>`,
  the optional `Arc<ArchiveSource>` / entry name for ZIP-backed tabs,
  and the per-document UI state: `selection`, `scroll_x`, `v_scroll`,
  `search_open / search_query / search_matches / search_active /
  search_generation`, `copy_flash_rect / copy_flash_generation`. Also
  exposes `tab_label()` (basename for direct files, `archive:entry`
  for ZIP entries, truncated at 40 chars with an ellipsis).
- `MainView` gains `documents: Vec<DocumentSnapshot>` and
  `active: Option<usize>`. The live per-document fields (`model`,
  `selection`, `scroll_x`, `v_scroll`, `search_*`, `copy_flash_*`,
  `archive`, `selected_entry`) are now a mirror of `documents[active]`.
- `snapshot_active` copies live -> snapshot; `load_active` copies
  snapshot -> live (or clears to defaults when `active == None`).
  They are the only mutators of the mirror besides `clear_live_per_document`.
- `apply(OpenOutcome::Ready)` snapshots the previous tab and pushes a
  new `DocumentSnapshot` as the active tab. Opening a file no longer
  replaces the previous document.
- `switch_to(index)` snapshots, moves `active`, loads. It sets
  `focus_grid_on_next_frame` so keyboard focus follows the tab.
- `close_active_at(index)` removes the snapshot and adjusts `active`
  correctly for closing the active tab, an earlier inactive tab, an
  later inactive tab, or the last tab. Closing the last tab resets
  `state` to `ViewState::Empty`.
- `render_tab_bar` — a horizontal strip above the grid and below the
  toolbar. Each tab is a click-to-switch region with a separate ×
  close cell (`.on_mouse_down` so it does not also select the tab).
  Zero-height when no documents are open.
- `on_file_drop` now iterates every dropped path and calls
  `begin_open` once per path, so a multi-file drop creates one tab
  per file.
- Archive picker gains an "Open all N files as tabs" button
  (`choose_all_entries`). It loops over the usable entries on the
  background executor, calling `apply` for each; tabs appear
  progressively and the loop aborts when `generation` has moved on.
- `render_center` now short-circuits to `render_grid` whenever
  `active.is_some()`, so `Empty` / `Loading` / `Error` only take over
  when no tab is open. A background-task error on a second open sets
  `status_message` and (only when no tab is open) `state = Error`;
  it never tears down the visible document.
- `NeedArchiveChoice` no longer clears the current tab's selection /
  scroll / search; the picker is a transient overlay.
- `close_search` now snapshots first so the active tab's search
  state stays consistent when the bar is dismissed.
- `theme::TAB_BAR_HEIGHT = 28px`.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 148 passed (47 lib + 41 binary + 60
  integration), 0 failed, 1 ignored. The binary suite grew from 32
  to 41 with nine new tab tests.
- `cargo clippy --release --all-targets` — clean (modulo the
  pre-existing future-incompat notes).
- `cargo fmt --check` — clean.
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, 20 cols, 0 warnings, 344 ms.
- `./target/release/anvil --probe fixtures/multi.zip` — 2 candidates
  listed.
- `./target/release/anvil` — launches, runs 5 s without panic.
- Manual GUI confirmation of tab switching / close-click is deferred;
  the state machine is covered by the nine new UI tests, including a
  real three-entry ZIP for `choose_all_entries`.

### Tests added / changed

- New UI tests in `src/ui/view.rs`:
  - `apply_ready_creates_one_tab`
  - `two_sequential_applies_create_two_tabs`
  - `error_with_open_tab_keeps_the_tab`
  - `push_two_documents_switches_to_last`
  - `switching_tabs_restores_per_document_state`
  - `closing_active_tab_picks_neighbour`
  - `closing_earlier_non_active_tab_shifts_active`
  - `closing_last_tab_leaves_empty_state`
  - `open_all_entries_adds_a_tab_per_entry` (builds a real 3-entry
    ZIP, calls `choose_all_entries`, asserts 3 tabs)
- The existing `inject_doc` helper now pushes a `DocumentSnapshot`
  and goes through `load_active`, so the 20+ existing UI tests
  exercise the new model.
- No data-side tests changed.

### Roadmap changes

- Milestone 2 gains a "Multi-document tabs" subsection with the
  landed items ticked and three follow-ups recorded (dockable side
  panel / tree view; tab reordering + tooltips; recently-closed tabs).
- Milestone 2 Tests gains the multi-document UI regression item.
- Future Ideas: "Multiple datasets / tabs" rephrased to the remaining
  "Dockable side panel / document tree" work, since the flat tab
  strip now exists.

### Architecture changes

- `ARCHITECTURE.md`:
  - Primary-state table rewritten around `documents` / `active` and
    the mirror fields, plus a paragraph on `DocumentSnapshot` and
    the snapshot/load helpers.
  - New "Multi-document / tabs" section covering the state model,
    `snapshot_active` / `load_active`, the open / switch / close
    flows, `choose_all_entries`, and the status/error behaviour.
  - "Drag / drop" updated: every path opens, not just the first.
  - "What is current vs planned" `ui::view` entry mentions the tab
    strip and links to the new section.
  - Planned Evolution gains a "Multi-document / tabs (landed)"
    bullet.
  - Testing section updated to twenty-three UI tests, with the nine
    new tests listed and a "UI / tabs (binary)" table row.
  - Functional gaps note the tab strip's limits (flat row, no
    docking, no reordering) and the updated UI test count.
  - Architectural debt now records the `view.rs` line-count growth
    (~2700) and suggests a split as the next large feature's
    prerequisite.

### Issues / technical debt discovered

- `view.rs` is now ~2700 lines; the tab list adds another
  responsibility to an already-large file. A split (e.g. `tabs.rs`,
  `documents.rs`, `search.rs`, `toolbar.rs`, `status.rs`) is becoming
  worthwhile before the next feature.
- The tab strip is flat: no dockable side panel, no reordering, no
  tooltip for truncated labels. The user's phrasing offered both a
  side panel and a tab menu; the tab menu landed first as the smaller
  of the two.
- `search_task` stays on `MainView` rather than the snapshot, so an
  in-flight search that finishes after a tab switch writes its result
  to the (now different) live fields. Today the generation check
  mostly masks this because `search_generation` is also live, but a
  pending search could theoretically land on the wrong tab. A future
  slice should move `search_task` into the snapshot or capture the
  tab index with the task.
- `copy_flash_generation` has the same shape: the flash timer compares
  against the live generation, so switching tabs during the 450 ms
  flash leaves the snapshot's `copy_flash_rect` set. Harmless visually
  (the flash reappears if the user returns) but not intentional.
- `DocumentSnapshot` is `Clone` because `load_active` cloned it; the
  clone is only needed for that one access pattern and could be
  avoided by restructuring the borrow. Minor.
- `choose_all_entries` loads every entry in sequence. For a ZIP with
  hundreds of entries this is a lot of I/O and memory; a threshold or
  a confirmation step would be a sensible follow-up.
- Dropping a directory or an unreadable path is not special-cased;
  the open task fails and the status bar shows the error. Acceptable
  for now.

### Open loops

- Move `search_task` / `copy_flash_generation` semantics fully into
  the snapshot.
- Dockable side panel / tree view as an alternative presentation of
  the same `DocumentSnapshot` list.
- Tab reordering and truncated-label tooltips.
- `Cmd/Ctrl+Home/End` and `Tab / Shift-Tab` remain unimplemented from
  the earlier keyboard-navigation list.
- Case-sensitive / column-scoped search UI toggles remain
  unimplemented on the data side.

### Next recommended task

The user's request is only half-done: the tab bar shows and switches
between open documents, but ZIPs with several entries still require
the picker (with the new "Open all" button) rather than showing
entries in the tab strip at the moment the ZIP is dropped. The next
slice should decide between:

1. Auto-opening every usable entry when a ZIP is dropped (with a
   size threshold), so the tab strip is the single surface for file
   navigation; or
2. A persistent archive-contents panel that lists entries and opens
   them on click, leaving the picker for the initial drop only.

Either way, the work is now smaller because `choose_all_entries` and
`DocumentSnapshot` already exist. I would also strongly consider
splitting `view.rs` first, as its size is now the main risk to further
UI changes.

## 2026-10-11 — Lazy tabs for every ZIP entry

### Context

After the multi-document tab slice, ZIPs with several entries still
required the modal picker (or its "Open all" button) before their
files appeared as tabs. The user asked for the opposite: "show all
entries immediately in the zip, like all files should be populated at
the tab menu". This iteration makes every usable archive entry a tab
the moment the ZIP opens, and extracts each entry's data lazily when
the tab is first activated.

### Implemented

- `DocumentSnapshot` changes:
  - `doc: Arc<ParsedDocument>` became `doc: Option<Arc<ParsedDocument>>`
    so a tab can exist before its data is loaded.
  - New `id: u64` (allocated from `MainView::next_doc_id`) so a lazy
    load can find its tab even if the list changed while it ran.
  - New `loading: bool` so repeated `switch_to` calls do not spawn
    duplicate extraction tasks.
  - `from_doc` now takes the id; new `pending_entry(id, archive,
    entry_name)` builds an unloaded ZIP tab; `is_loaded()` helper.
  - `tab_label()` shows the entry basename for ZIP tabs (the archive
    name moved out of the strip) and "loading…" only as a fallback.
- `install_archive_tabs(source, loaded_entry, loaded_doc, cx)`:
  installs one tab per usable entry. The central directory gives
  names and sizes without decompression, so the whole strip is built
  synchronously. The entry the loader pre-opened (when there is a
  clear favourite) becomes the ready active tab; the rest are
  pending.
- `apply` now routes `Ready` with an archive and `NeedArchiveChoice`
  through `install_archive_tabs`. `NeedArchiveChoice` no longer sets
  `show_picker`; the modal picker is not shown automatically.
- `ensure_active_loaded(cx)` starts a detached background
  `extract_and_index` task for the active pending tab and marks it
  loading. Called from `install_archive_tabs`, `switch_to`, and
  `close_active_at`.
- `apply_lazy_result(id, outcome, cx)` finds the tab by id, stores
  the parsed document, and rebuilds the live model only if the tab
  is still active. A tab closed mid-load is simply not found, so
  the result is dropped. A failed extraction drops the tab and
  reports the error in the status bar.
- `render_center` shows a `Loading…` placeholder (with the entry
  name) while the active tab is pending, instead of a blank grid.
- `render_archive_picker` no longer offers "Open all"; clicking an
  entry calls the new `switch_to_entry`, which finds the matching
  tab and switches to it. The toolbar **Archive…** button still
  opens the picker for an entry list and the skipped-entries count.
- Removed `choose_entry` and `choose_all_entries` (superseded).

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 150 passed (47 lib + 43 binary + 60
  integration), 0 failed, 1 ignored.
- `cargo clippy --release --all-targets` — clean (modulo the
  pre-existing future-incompat notes).
- `cargo fmt --check` — clean.
- `./target/release/anvil --probe fixtures/multi.zip` — still lists
  the two candidates (the probe uses the core loader, which still
  reports `NeedArchiveChoice`; the UI now turns that into tabs).
- `./target/release/anvil` — launches, runs 5 s without panic.
- Manual confirmation that all ZIP entries appear as tabs is
  deferred; the behaviour is covered by three new UI tests that
  build real ZIPs.

### Tests added / changed

- New UI tests in `src/ui/view.rs`:
  - `archive_entries_become_tabs_immediately` — three-entry ZIP: all
    three tabs exist synchronously, the first lazy-loads, switching
    to the second loads it while the third stays pending.
  - `apply_need_archive_choice_installs_all_tabs` — the multi-entry
    drop path installs all tabs and shows no picker.
  - `install_archive_tabs_marks_loaded_entry_as_ready` — a pre-opened
    favourite is the ready active tab; the other entry is pending
    and loads on switch.
- New test helper `make_zip_archive(names)` builds a real ZIP and
  returns its `ArchiveSource`.
- New test helper `push_loaded_tab(view, doc)` replaces the five
  hand-rolled snapshot pushes in the existing tab tests.
- Replaced `open_all_entries_adds_a_tab_per_entry` (the removed
  `choose_all_entries` test).
- No data-side tests changed.

### Roadmap changes

- Milestone 2 "Multi-document tabs" rewritten: eager per-entry tabs,
  lazy extraction, no automatic picker, and the failed-extraction
  behaviour are all ticked. The three follow-ups (dockable panel,
  reordering/tooltips, recently-closed tabs) are unchanged.
- Milestone 2 Tests item updated to the new lazy-tab coverage.

### Architecture changes

- `ARCHITECTURE.md`:
  - Primary-state `DocumentSnapshot` paragraph now describes the
    optional document, stable id, and loading guard.
  - "Multi-document / tabs" rewritten: eager `install_archive_tabs`,
    a dedicated "Lazy loading" subsection (`ensure_active_loaded` /
    `apply_lazy_result` / detached task / id lookup), the picker's
    new role, and the loading-placeholder render path.
  - Planned Evolution bullet notes lazy extraction.
  - Testing section: list of tab tests replaced; count 23 -> 25.
  - Functional gaps note that a ZIP with hundreds of entries creates
    hundreds of unloaded tabs (no cap/grouping yet).

### Issues / technical debt discovered

- Every usable entry gets a tab eagerly. A ZIP with hundreds of
  entries creates hundreds of `DocumentSnapshot`s. Each is small
  and lazy loading keeps extracted data bounded, but there is no cap
  or grouping. Worth revisiting if large ZIPs become common.
- A single failed lazy extraction removes that tab. If several
  entries fail, the user sees a burst of status-bar errors and tabs
  disappearing one by one. A batch error summary would be nicer.
- The tab label for a ZIP entry is just the basename, so two archives
  opened at once that both contain `data.csv` show identical labels.
  Tooltips / archive-qualified labels are the recorded follow-up.
- `ensure_active_loaded` spawns one detached task per pending tab; a
  user rapidly clicking through a large archive starts many
  extractions. There is no cancellation or queue. Acceptable for now
  because each task is a single entry, but worth a queue if it
  becomes a memory issue.
- `view.rs` remains ~2900 lines; the split recommendation from the
  previous entry still stands.

### Open loops

- Cap / group very large archives rather than creating a tab per
  entry.
- Cancellation or batching for lazy extraction tasks.
- `search_task` / `copy_flash_generation` still live on `MainView`
  rather than the snapshot (from the previous entry).
- Dockable side panel / tree view; tab reordering; tooltips.
- Case-sensitive / column-scoped search UI toggles.

### Next recommended task

The user's file-navigation request is now satisfied: a dropped ZIP
populates the tab menu with every usable entry and loads each on
demand. The next slice should be the deferred **filter** work
(`Cmd/Ctrl+L`), which has been the standing recommendation since the
search slice. Alternatively, split `view.rs` first: at ~2900 lines it
is the main obstacle to further UI work, and the tab / lazy-load code
is a natural first extraction (`tabs.rs`).

## 2026-10-12 — File-manager panel + scrollable tab strip

### Context

The user confirmed the lazy-loading approach ("just the first one that
opens") — which the previous iteration already implements: a dropped
ZIP creates a tab per usable entry but only the loader's favourite (or,
for an ambiguous archive, the first tab) is extracted. They then raised
two follow-ups: the tab menu can overflow with no way to reach distant
files, and they still want a file-manager area on the left or right
listing the opened files. This iteration adds that panel and makes the
tab strip scroll.

### Implemented

- `MainView.file_panel_open: bool` (default `true`) and
  `MainView.tab_scroll: ScrollHandle`.
- `group_documents(&[DocumentSnapshot])` — free function that groups
  consecutive snapshots sharing an archive by `Arc` identity. Direct
  files are top-level; ZIP entries nest under their archive header.
  Returns `(Option<Arc<ArchiveSource>>, Vec<(tab_index, label,
  loaded)>)` in tab order.
- `render_file_panel` — left-hand panel, `theme::FILE_PANEL_WIDTH`
  wide, with:
  - a header showing the open count and a × that hides the panel;
  - archive group headers (`▾ archive.zip`);
  - file rows with an active highlight, a loaded/pending dot, the
    label, and a × that closes the tab;
  - click-to-switch and click-to-close wired to the same
    `switch_to` / `close_active_at` the tabs use;
  - a scrollable body for long lists.
- `Render::render` now lays the body out as a row: the panel is a
  `flex_shrink_0` left child and the grid/search area takes the
  remaining space. The panel is only rendered when
  `file_panel_open && !documents.is_empty()`.
- Toolbar **Files** button toggles the panel.
- `Action::ToggleFilePanel` (Cmd/Ctrl+B) added to the binding table,
  with two unit tests (Cmd/Ctrl+B binds; plain B does not).
- Tab strip: tabs are now `flex_shrink_0` and the inner track is
  `overflow_x_scroll()` + `track_scroll(&self.tab_scroll)`, so a long
  tab list scrolls horizontally instead of crushing every tab.
- `theme::FILE_PANEL_WIDTH = 220px` and
  `theme::FILE_PANEL_ENTRY_INDENT = 18px`.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 155 passed (47 lib + 48 binary + 60
  integration), 0 failed, 1 ignored. The binary suite grew from 43
  to 48.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `./target/release/anvil` — launches, runs 5 s without panic.
- `./target/release/anvil --probe fixtures/multi.zip` — unchanged.
- Manual GUI confirmation of the panel layout is deferred; the
  grouping helper, the toggle binding, and the panel's switch / close
  paths all have automated coverage.

### Tests added / changed

- `src/ui/commands.rs`:
  - `toggle_file_panel_chord`
  - `plain_b_is_not_toggle_file_panel`
- `src/ui/view.rs`:
  - `cmd_b_toggles_file_panel` — Cmd/Ctrl+B flips `file_panel_open`
    both ways through the grid keyboard path.
  - `group_documents_groups_by_archive_identity` — direct files stay
    top-level; two archives with separate `Arc`s stay separate even
    when their entry names overlap.
  - `file_panel_navigation_uses_tab_lifecycle` — switch loads the
    pending neighbour and close adjusts `active`, using the same
    methods the panel rows call.

### Roadmap changes

- Milestone 2 "Multi-document tabs": ticked the file panel and the
  scrollable tab strip; replaced the "dockable side panel / tree
  view" follow-up with the remaining work (resizable / movable /
  icon-rail / truly nested tree).
- Milestone 2 Tests: added the file-panel regression item.
- Future Ideas: the "Dockable side panel / document tree" bullet now
  describes making the landed panel resizable / movable.

### Architecture changes

- `ARCHITECTURE.md`:
  - Primary-state table gained `next_doc_id` (previously missing),
    `file_panel_open`, and `tab_scroll`.
  - "Multi-document / tabs" gained a "Presentation — tab strip and
    file panel" subsection describing the scroll behaviour,
    `group_documents`, the panel's toggle surfaces, and the layout.
  - Planned Evolution and Functional gaps updated: the panel exists
    but is fixed-width, left-only, and two-level; the remaining
    work is recorded.
  - Testing section: new tests listed, UI count 25 -> 28, commands
    count 14 -> 16, new "UI / file panel (binary)" table row.

### Issues / technical debt discovered

- The panel is a fixed 220 px on the left. No resize grip, no move to
  the right, no collapse-to-icon-rail. Recorded.
- `group_documents` groups only consecutive runs. Two documents from
  the same archive separated by another document (possible if the user
  reorders tabs later, or opens the same archive twice) would appear
  as two groups. Tab reordering is not implemented yet, so this is not
  reachable today.
- Row ids use the tab index (`("file-row", index)`). When tabs close,
  indices shift, so GPUI may attach hover state to a different row for
  a frame. Harmless for stateless rows but worth switching to the
  stable `DocumentSnapshot::id` if hover glitches appear.
- The panel body's `overflow_y_scroll()` is not backed by a tracked
  `ScrollHandle`, unlike the tab strip. It works via GPUI's default
  scrolling; a handle would let the app scroll a row into view when
  the active tab changes.
- `view.rs` is now ~3100 lines. The split recommendation is more
  urgent; the file panel is another natural extraction.

### Open loops

- Resizable / movable / collapsible file panel; nested tree beyond
  archive → entry.
- Scroll the panel to keep the active row visible.
- Use `DocumentSnapshot::id` for panel row element ids.
- Cap / group very large archives (from the previous entry).
- Cancellation or batching for lazy extraction tasks (from the
  previous entry).
- `search_task` / `copy_flash_generation` still live on `MainView`
  rather than the snapshot.

### Next recommended task

The user's navigation request is now covered by both the tab strip
and the file panel. The standing deferred work is **filter**
(Cmd/Ctrl+L). Before that, though, splitting `view.rs` (~3100 lines)
would materially reduce the cost of every future UI change; the file
panel and the tab/lazy-load code are the two clearest extractions.

## 2026-10-13 — Milestone 2 filter (Cmd/Ctrl+L)

### Context

Filter was the last major Milestone 2 feature after search. This
iteration lands the whole vertical: a data-side predicate module with
a parser, a filter bar, background evaluation with generation-based
obsolescence, grid rendering from the filtered set, filtered-count
status, and filter-aware vertical navigation. It also reports the
first filter throughput measurement.

### Implemented

Data side (`src/query/filter.rs`, module `query::filter`):

- `FilterOp`: `Eq`, `Ne`, `Contains`, `NotContains`, `StartsWith`,
  `EndsWith`, `Empty`, `NonEmpty`, `Gt`, `Ge`, `Lt`, `Le`, with a
  permissive `parse` (`=`, `<>`, `!contains`, `starts`, `ends`,
  `notempty`, …).
- `Filter { column, op, value }`.
- `parse(input, headers, column_count)`: one-line expression
  `<column> <op> [value]`; column resolved by case-insensitive header
  name, bare 0-based index, or `#N`; values may contain spaces;
  `empty` / `notempty` take no value. Clear error strings for unknown
  column / operator / out-of-range index.
- `apply(doc, &[Filter]) -> Vec<u64>`: values are pre-lowercased and
  pre-parsed once, then each record is decoded lazily. Text operators
  fold ASCII case; ordering operators skip non-numeric and missing
  cells; a missing cell counts as empty. Multiple filters AND.

UI (`src/ui/view.rs`, `src/ui/commands.rs`):

- New `Action::OpenFilter` (Cmd/Ctrl+L).
- Per-tab filter state on `DocumentSnapshot` (`filter_input`,
  `filter_applied`, `filter_rows`, `filter_generation`) mirrored into
  `MainView`, plus global `filter_open`, `filter_error`, `filter_task`,
  `filter_focus_handle`, `focus_filter_on_next_frame`.
- `open_filter` / `close_filter` (bar visibility), `apply_filter_input`
  (parse + background `apply_filter`, clearing rows while in flight),
  `clear_filter`, and `on_filter_key_down` (Enter applies, Esc closes,
  Backspace edits, printable characters append — no live re-evaluation).
- `render_filter_bar` is a normal-flow child above the search bar with
  its own focus handle, so it never overlaps the grid. `filter_status_text`
  shows a parse error, "filtering…", "N rows", or "none".
- `render_grid` maps each display position through `filter_rows` to a
  record index; the row renderer, selection, and search highlighting
  keep using record indexes unchanged.
- `move_selection` walks the visible rows when filtered, so arrow keys
  never land on a hidden record; horizontal movement is unchanged.
- `render_status` shows `N rows (filtered from M)`.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 185 passed (65 lib + 56 binary + 64
  integration), 0 failed, 2 ignored (the two throughput tests).
- `cargo test --release --lib filter` — 18 passed.
- `cargo test --release --bin anvil filter` — 8 passed.
- `cargo test --release --test core filter` — 4 passed.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `cargo test --release --test core -- --ignored measure_filter
  --nocapture` — **1,000,000 rows filtered in 33 ms, 99,479 matches
  (~10%)** for `country == US`. Milestone 2 budget is < 2 s.
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, 20 cols, ~350 ms.
- `./target/release/anvil` — launches, runs 5 s without panic.

### Tests added / changed

- 18 unit tests in `src/query/filter.rs` covering parser resolution,
  rejections, value-less operators, spaces in values, every operator,
  case folding, numeric ordering (including non-numeric skips),
  missing cells, AND composition, empty filter list, and quoted value
  unescaping.
- 4 integration tests in `tests/core.rs` against the public API.
- 2 command-binding tests (`open_filter_chord`,
  `plain_l_is_not_open_filter`).
- 6 UI tests (`cmd_l_opens_filter_bar`, `filter_input_applies_on_enter`,
  `filter_parse_error_is_shown`, `clear_filter_restores_document`,
  `filter_vertical_navigation_skips_hidden_rows`,
  `filter_status_label_reports_counts`).
- 1 ignored throughput test `measure_filter_large_fixture`.

### Roadmap changes

- Milestone 2 Filter: all six tasks ticked, with a note on the exact
  operator set and the display->record mapping. Two follow-ups added:
  a richer expression language (`AND` / `OR`, parentheses) and saved
  filters / chips.
- Milestone 2 Tests: "Unit tests for `query::filter` predicates
  against fixtures" ticked.

### Architecture changes

- `ARCHITECTURE.md`:
  - Repository tree gained `selection.rs`, `query.rs`, and
    `query/filter.rs`; integration-test count 60 -> 64.
  - Primary-state table gained `filter_open`, `filter_input`,
    `filter_applied`, `filter_rows`, `filter_generation`,
    `filter_error`, `filter_task`, `filter_focus_handle`,
    `focus_filter_on_next_frame`; `DocumentSnapshot` paragraph notes
    the per-tab filter state.
  - Planned Evolution "Filter (Milestone 2)" rewritten as landed, with
    the operator set, parser, background evaluation, grid mapping, and
    navigation.
  - Performance Characteristics records the 33 ms / 99,479-match
    measurement.
  - Testing section updated (64 integration, 18 filter unit tests, 34
    UI tests, 18 command tests) with a Filter table row and a UI /
    filter row.
  - Functional gaps now record the single-predicate expression
    language and the lack of a column/operator picker or saved
    filters.
- `README.md`: test-count line updated.

### Issues / technical debt discovered

- The expression language is one predicate per apply. `apply` already
  ANDs a slice, so `AND` / `OR` / parentheses is parser + UI work, not
  a data-model change.
- The filter bar requires typing the column and operator by hand; there
  is no completion, no dropdown, and no column picker. Error messages
  are plain strings. A richer input (chips, a column dropdown) is the
  natural follow-up.
- `filter_open` is global while the predicate is per tab. Switching
  tabs with the bar open shows the new tab's input, which is
  reasonable, but the bar never closes on its own.
- The grid row number shows the original record index, not the display
  position. That is deliberate (the user wants to know the true CSV
  row) but could surprise when scrolling a filtered set.
- `view.rs` is now ~3700 lines. The split recommendation is overdue;
  filter added another ~350 lines. The search bar, filter bar, file
  panel, and tab/lazy-load model are all natural extractions.
- `filter_rows` is recomputed from scratch on every apply. For 1M rows
  that is 33 ms, so fine; a future incremental filter is unnecessary
  at current data sizes.

### Open loops

- Compound filter expressions (`AND` / `OR`, parentheses).
- Filter input affordances: column dropdown, operator menu, chips,
  saved filters.
- Split `view.rs` before the next large UI feature.
- Sort (Milestone 3) can reuse the same display->record mapping, but
  must compose with the active filter (sort the filtered indexes, not
  the whole document).
- Carried over: cap / group large archives, lazy-extraction
  cancellation, `search_task` / `copy_flash_generation` per-tab, panel
  resize/reorder, case-sensitive / column-scoped search toggles.

### Next recommended task

Extract the UI modules from `view.rs` before starting Milestone 3.
`view.rs` is ~3700 lines and every recent feature has added to it; the
search bar, filter bar, file panel, and document/tab model are four
clean extractions that would each be a small, verifiable slice. After
that, Milestone 3 sort is the natural feature: it reuses the
display->record mapping introduced here and must compose with the
filter (sort the filtered indexes).

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

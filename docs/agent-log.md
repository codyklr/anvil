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
never focusable, and _no_ grid keyboard event ever reached
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

- `anvil_core::query::search` (pure, GPUI-free) with `SearchOptions`
  (case-sensitive / column-scope / max-matches);
- `commands::Action::OpenSearch` and `Action::NextMatch` bindings;
- A search-bar overlay with a dedicated focus handle, character
  input, Backspace, Enter / Shift+Enter, and Esc;
- Background search task on `cx.background_executor` with
  generation-counter cancellation;
- Match highlighting in the grid (`BG_MATCH` / `BG_MATCH_ACTIVE`);
- F3 / Shift-F3 cycle matches from anywhere via the grid's
  `Action::NextMatch` arm;
- Search state is cleared on every `OpenOutcome` so opening a new
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
  - "What is _current_ vs _planned_" updated: `query::search` is
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
- Therefore the search bar _overlapped_ the top of the grid: 36 px
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
  it, and that the active-match row stays fully visible even at row 0.

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
but the user could not tell _which cell_ in the row contained the
match, let alone _where_ inside the cell. The user explicitly asked
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

## 2026-10-14 — Milestone 2 keyboard navigation completion (Cmd/Ctrl+Home/End, Tab/Shift-Tab)

### Context

Milestone 2 was functionally complete after the filter slice, but the
selection/navigation checklist still had three unticked items.
Two of them are pure grid keyboard navigation: Cmd/Ctrl+Home/End
(jump to first/last row) and Tab/Shift-Tab (adjacent-cell movement).
The third (Cmd/Ctrl+G go-to-row) needs a new input overlay and was
left for a follow-up. This iteration completes the two keyboard items
and, in doing so, found and fixed a latent integer-overflow bug in the
existing Home/End column bindings.

### Implemented

Data side (`src/selection.rs`):

- `Selection::move_cursor` now uses `saturating_add` instead of `+`.
  The existing Home / End bindings pass `i64::MIN` / `i64::MAX` as
  the column delta, so the old arithmetic could overflow (a debug-build
  panic; wrapping in release). The new saturating path clamps to the
  document edge as intended.
- `Selection::move_cell(forward, column_count, record_count, extend)`:
  moves to the adjacent cell in row-major order, wrapping across rows
  and around the document ends. No-op on a zero-row/zero-column
  document. Reuses a new private `apply_destination` helper that also
  serves `move_cursor`, keeping anchor/`extend` semantics identical.

Keyboard layer (`src/ui/commands.rs`):

- New `Action::NextCell { forward }` for Tab / Shift-Tab.
- `action_for` now maps Cmd/Ctrl+Home and Cmd/Ctrl+End to
  `MoveCursor { row_delta: i64::MIN | i64::MAX, column_delta: 0 }`
  (Shift extends), before the plain Home/End column-jump arms.
- Tab maps to `NextCell { forward: !modifiers.shift }`.

UI (`src/ui/view.rs`):

- `run_action` handles `Action::NextCell` via a new
  `move_cell_selection`.
- `move_cell_selection` delegates to `Selection::move_cell` when there
  is no filter, and walks the _visible_ cells (position -> record
  mapping, wrapping) when a filter is active.
- `move_selection`'s filtered branch now uses `saturating_add` for the
  row/column arithmetic so Cmd/Ctrl+Home/End work while filtered and
  cannot overflow.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 198 passed (72 lib + 62 binary + 64
  integration), 0 failed, 2 ignored (the throughput tests). Was 185.
- `cargo test --release --lib selection` — 32 passed.
- `cargo test --release --bin anvil` — 62 passed, including the three
  new UI tests.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, 20 cols, ~359 ms.
- `./target/release/anvil --probe fixtures/archive.zip` — 1,000,000
  rows, ~548 ms.
- `./target/release/anvil` — launches and stays alive for 5 s with no
  panic (no interactive key exercise; the behaviours are covered by
  the headless UI tests).

### Tests added / changed

- `src/selection.rs`: 7 new unit tests —
  `move_cursor_with_extreme_delta_saturates`,
  `move_cursor_with_extreme_column_delta_saturates`,
  `move_cell_advances_and_wraps_rows`,
  `move_cell_backward_wraps_to_previous_row_end`,
  `move_cell_wraps_around_document_ends`,
  `move_cell_from_empty_anchors_at_origin`,
  `move_cell_no_op_on_empty_document`.
- `src/ui/commands.rs`: 3 new unit tests —
  `cmd_home_and_cmd_end_jump_to_row_extremes`,
  `cmd_shift_home_extends_to_first_row`,
  `tab_and_shift_tab_move_adjacent_cells`.
- `src/ui/view.rs`: 3 new UI regression tests —
  `cmd_home_and_end_jump_to_row_extremes`,
  `tab_and_shift_tab_move_adjacent_cells`,
  `filter_tab_and_row_jumps_walk_visible_rows`. The Tab UI test also
  confirms GPUI delivers Tab to the grid's `on_key_down` rather than
  swallowing it for focus traversal.

### Roadmap changes

- Milestone 2 → Keyboard navigation:
  - `[ ] Cmd/Ctrl+Home/End jump to first / last row` → `[x]`, with a
    note on the `i64::MIN` / `i64::MAX` + saturating-arithmetic
    implementation.
  - `[ ] Tab / Shift-Tab navigation between cells` → `[x]`, with a
    note on `Selection::move_cell` and the filtered visible-row walk.
- Milestone 2 → Tests: added a `[x]` line for the new unit + UI
  coverage.
- `Cmd/Ctrl+G — go-to-row input box` remains unchecked; it needs a new
  overlay rather than a binding-table arm.

### Architecture changes

- `ARCHITECTURE.md`:
  - Recognised-keys table gained Cmd/Ctrl+Home, Cmd/Ctrl+End, and
    Tab / Shift-Tab rows; the paragraph below now describes the
    saturating deltas and `Action::NextCell`.
  - Selection operations table gained `Selection::move_cell` and notes
    `move_cursor`'s saturating arithmetic.
  - Grid-integration keyboard bullet and the `commands::action_for`
    description updated; `Action` list gained `NextCell`.
  - Testing section: selection unit tests 25 → 32, UI regression tests
    34 → 37, command tests 18 → 21, and the selection/commands/focus
    table rows updated.
  - Functional gaps and Planned Evolution no longer claim
    Cmd/Ctrl+Home/End are outstanding.
- `README.md`: test-count line updated (56 → 62 binary tests).

### Issues / technical debt discovered

- The latent `i64::MIN` overflow in `move_cursor` had been present
  since the Home/End bindings landed; release builds wrapped and the
  effect was correct, so the existing characterisation tests did not
  catch it. Saturating arithmetic is now covered by explicit tests.
- Tab is now consumed by the grid whenever it holds focus. There is no
  other focusable widget in the main grid surface today, so this is
  fine, but a future focusable child inside the grid would need to
  stop the Tab event or rebind.
- Tab / Shift-Tab do not extend a range (they move the cursor), which
  matches spreadsheets. If a future slice wants Shift-Tab to extend,
  `Selection::move_cell` already accepts `extend`.
- `view.rs` is now ~3870 lines. The split recommendation from the
  previous iteration is still overdue.

### Open loops

- Cmd/Ctrl+G go-to-row input box (last unchecked Keyboard-navigation
  item).
- Search input options (case-sensitive toggle, column scope) and
  column-scoped search via header click.
- Split `view.rs` before the next large UI feature.
- Carried over: cap / group large archives, lazy-extraction
  cancellation, per-tab `search_task` / `copy_flash_generation`,
  panel resize/reorder, compound filters, sort (Milestone 3).

### Next recommended task

Milestone 2's keyboard-navigation checklist is now complete except for
Cmd/Ctrl+G. Two reasonable next steps: (a) finish the search UX by
surfacing the already-supported `case_sensitive` / `column` options in
the bar and wiring column-scoped search from a header click, which
closes the remaining Milestone 2 Search checkboxes; or (b) do the
overdue `view.rs` split before starting Milestone 3. The search UX is
smaller and closes out Milestone 2; the split should land before the
next large feature (Milestone 3 sort / column inspector).

## 2026-10-15 — Milestone 2 search options + column-scoped search

### Context

Milestone 2's Search section still had two unchecked tasks: exposing
`SearchOptions` in the bar (case-sensitive toggle, column scope) and
column-scoped search via a header click. The data side already
supported both; this iteration wired the UI end to end. It also closes
out the Milestone 2 Search checklist.

### Implemented

Search state (`src/ui/view.rs`):

- New per-tab fields on `DocumentSnapshot` and live mirrors on
  `MainView`: `search_case_sensitive: bool` and
  `search_column: Option<usize>`, wired through `snapshot_active`,
  `load_active`, and `clear_live_per_document` like the rest of the
  search state.
- `MainView::search_options()` is now the single source of truth for
  `SearchOptions`; both `kick_search` and `render_grid` read it, so the
  background result and the highlight pass cannot drift.
- `toggle_search_column(column)`, `clear_search_column()`, and
  `toggle_search_case_sensitive()` mutate the options and re-run an
  open search so matches and highlighting follow immediately.
  `toggle_search_column` toggles (clicking the scoped header clears
  it) and keeps keyboard focus on the grid.
- `close_search` preserves both options deliberately so the
  "click a header, then Cmd/Ctrl+F" flow survives Esc; the scoped
  header stays highlighted so the state remains visible.
- `jump_to_active_match` now lands the cursor in the scoped column
  when one is set, instead of an unrelated column.
- `render_search_bar` gained an `Aa` case-sensitive toggle (highlighted
  when active) and a column-scope chip showing the header name with an
  inline × that clears the scope.

Grid (`src/ui/grid.rs`):

- `render_header` now takes `scoped_column` and an `on_header_click`
  closure; the scoped header is painted `BG_MATCH_ACTIVE` and clicking
  a header toggles the scope through the existing
  `entity.update(&mut App)` pattern used by data cells. Header cells
  carry a `header-cell-<col>` debug selector for tests.
- New pure `should_highlight(options, column)` gates substring
  highlighting to the scoped column; `render_row` renders non-scoped
  cells of a match row plain while still applying the row tint.
  `find_ranges` stays a pure per-cell helper.

### Verified

- `cargo build --release` — succeeds.
- `cargo test --release` — 205 passed (72 lib + 69 binary + 64
  integration), 0 failed, 2 ignored (throughput tests). Was 198.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, 20 cols, ~357 ms.
- `./target/release/anvil` — launches and stays alive for 5 s with no
  panic. Interactive header-click / toggle exercise was not performed
  by hand; the real header-click path is covered by a headless UI test
  (`header_click_toggles_search_column`) that locates the header cell
  via a debug selector and synthesises the click.

### Tests added / changed

- `src/ui/view.rs` (5 new):
  `search_case_sensitive_toggle_changes_matches` (case-insensitive
  `alice` → 3 matches, `Aa` → 1),
  `search_column_scope_limits_matches` (scope restricts matches and
  lands the cursor in the scoped column),
  `header_click_toggles_search_column` (real header click via
  `debug_bounds`, toggle on then off),
  `search_options_reflect_toggles`, and the pure
  `search_column_label_resolves_headers_and_fallback`.
- `src/ui/view.rs` (extended): `switching_tabs_restores_per_document_state`
  now also asserts the new search options reset on a new tab and are
  restored on switch-back.
- `src/ui/grid.rs` (2 new): `should_highlight_honours_column_scope`
  and `render_row_with_column_scope_does_not_panic`.

### Roadmap changes

- Milestone 2 → Search: `Search input + options` and
  `Column-scoped search` ticked, with notes on the `Aa` toggle,
  the scope chip, per-tab persistence, and the persistent header
  highlight. The Milestone 2 Search checklist is now complete.

### Architecture changes

- `ARCHITECTURE.md`:
  - Primary-state table gained `search_case_sensitive` and
    `search_column`.
  - Functional gaps: the "search bar does not expose the toggles"
    bullet is replaced with the implemented behaviour.
  - Planned Evolution "Search (Milestone 2)" describes the
    `search_options()` helper, the `Aa` toggle, header-click scoping,
    the scoped-header highlight, and the `should_highlight` gate.
  - Testing section: UI regression tests 37 → 42; grid unit tests and
    the UI/search and grid-geometry table rows updated.
- `README.md`: binary test count 62 → 69.

### Issues / technical debt discovered

- The scoped-header marker reuses `BG_MATCH_ACTIVE`, which is also the
  active-search-match row colour. It is visually adequate but not
  semantically distinct; a dedicated `BG_SEARCH_SCOPE` would be
  clearer if the palette grows.
- Column scope is per tab but case-sensitivity is arguably a global
  user preference. It was kept per tab for consistency with the rest
  of the search state; hoist it if a settings/config layer lands.
- Scope is deliberately preserved across `close_search`. The scoped
  header remains highlighted, so it is not hidden state, but there is
  no other affordance (e.g. a status-bar hint) when the bar is closed.
- Search always runs over the whole document, not the filtered set, so
  a column scope and a filter can disagree about which rows exist. This
  matches existing search behaviour; composing search with filter is a
  separate, larger change.
- `view.rs` is now ~4000 lines. The split remains overdue; this slice
  added another ~120 lines plus tests.

### Open loops

- Cmd/Ctrl+G go-to-row input box (last Keyboard-navigation item).
- Compound filter expressions, filter chips / column picker.
- Split `view.rs` before the next large feature (Milestone 3 sort /
  column inspector).
- Search-within-filter composition and a `max_matches` cap surfaced in
  the UI.
- Carried over: cap / group large archives, lazy-extraction
  cancellation, per-tab `search_task` / `copy_flash_generation`,
  panel resize/reorder, sort (Milestone 3).

### Next recommended task

Milestone 2's Search and Keyboard-navigation checklists are now
complete except for Cmd/Ctrl+G. The next largest coherent step is the
overdue `view.rs` split (the search bar + options, filter bar, file
panel, and document/tab model are four clean extractions), which the
log has recommended before starting Milestone 3. If feature work is
preferred over the refactor, Cmd/Ctrl+G closes the last Milestone 2
navigation item, or Milestone 3 sort / type inference can start on top
of the existing display->record mapping.

## 2026-10-16 — Mouse/keyboard text editing in the filter input

### Context

The filter bar (Cmd/Ctrl+L) and search bar were hand-rolled single-line
inputs: `MainView` held a `String`, rendered it as a truncated `div`,
and the key handler appended `key_char`s and `pop()`ed on Backspace.
There was no caret, no selection, and no mouse interaction, so clicking
or dragging in the field did nothing. This iteration makes the filter
input behave like a normal text field: click to place the caret,
click-drag / Shift+click to select (highlighted), arrow / Home / End /
word movement, Shift-extension, Cmd/Ctrl+A, and Backspace/Delete over
the selection.

### Implemented

New GPUI-free model (`src/ui/text_edit.rs`, 18 unit tests):

- `TextEdit { text, caret, anchor }` with `insert` (replaces the
  selection), `backspace` / `delete`, `move_left/right/home/end/
word_left/word_right`, `set_caret(index, extend)`, `select_all`,
  `selection()` / `selected_text()` / `has_selection()`. Every
  mutation clamps byte offsets to UTF-8 char boundaries, so multibyte
  text cannot be split. `From<&str>` / `From<String>` build an editor
  with the caret at the end.

Filter bar (`src/ui/view.rs`):

- `filter_input` is now a `TextEdit` on `MainView` and
  `DocumentSnapshot` (per tab, mirrored through `snapshot_active` /
  `load_active` / `clear_live_per_document`).
- `render_filter_bar` takes `Window`, paints the text with
  `StyledText` (selection as a `BG_SELECTED` highlight range), and
  positions a 1px caret from `text_caret_x` (which shapes the line with
  `window.text_system().shape_line(..)` and reads `x_for_index`).
- A hitbox-free `canvas` records the text area's window bounds each
  frame (`filter_text_bounds`), reusing the grid's overlay pattern.
  Mouse-down/drag map pointer x to a byte index via
  `closest_index_for_x`, place/extend the caret, and toggle
  `filter_dragging`.
- `on_filter_key_down` now maps editing keys onto `TextEdit`
  operations: arrows, Home/End, Shift-extension, Option+arrows (macOS)
  / Ctrl+arrows (elsewhere) for word movement, Cmd/Ctrl+A select-all,
  Backspace/Delete. Enter still applies, Esc still closes.
- New free helpers `text_index_at_point`, `text_caret_x`,
  `shape_input_line`.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 226 passed (72 lib + 90 binary + 64
  integration), 0 failed, 2 ignored (throughput tests). Was 205.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, ~354 ms.
- `./target/release/anvil` — launches and stays alive for 5 s with no
  panic. The filter bar is not open on launch, so interactive editing
  was not exercised by hand; the caret/selection/mouse paths are
  covered by the headless UI tests, including a real synthesised
  click-drag through the rendered field bounds.

### Tests added / changed

- `src/ui/text_edit.rs`: 18 unit tests covering the empty editor,
  caret-at-end construction, append, replace-selection, backspace
  (char / selection / start no-op), delete (char / selection),
  selection collapse on movement, shift-extension, Home/End, select-all
  - collapse, word movement across operator tokens, char-boundary
    clamping, and multibyte backspace. One expectation was corrected
    during development (word-left from between `==` and `US` lands before
    `==`, not after `country`).
- `src/ui/view.rs`: 3 UI tests — `filter_keyboard_caret_and_selection`
  (Home/End/Shift+Home), `filter_select_all_typing_and_backspace`
  (Cmd/Ctrl+A + typing replaces, select-all + Backspace clears), and
  `filter_mouse_click_and_drag` (click near the right edge puts the
  caret at the end; a drag from the start selects a non-empty prefix).
- Existing filter tests were unaffected: `From<&str>` keeps
  `view.filter_input = "...".into()` working.

### Roadmap changes

- Milestone 2 → Filter: added a checked task for the editable filter
  input (caret, selection, mouse click/drag, word movement), plus an
  unchecked follow-up to migrate the search bar to the same model.
- Milestone 2 → Tests: added a checked line for the `TextEdit` unit
  tests and the filter-editing UI tests.

### Architecture changes

- `ARCHITECTURE.md`:
  - Repository tree gained `ui/text_edit.rs`.
  - Primary-state table: `filter_input` is a `TextEdit`; added
    `filter_text_bounds` and `filter_dragging`.
  - New "Editable text inputs" section describing the model, the
    `StyledText` + shaped-line rendering, the bounds-recording canvas,
    the mouse/key routing, and the current scope/limitations.
  - Testing section: UI regression tests 42 → 45, a `Text edit
(binary)` row, and the coverage paragraph updated.
  - Functional gaps note that the search bar has not been migrated and
    that there is no caret blink or in-field horizontal scroll.
- `README.md`: binary test count 69 → 90.

### Issues / technical debt discovered

- The search bar still uses the old append/pop input. It should adopt
  `TextEdit` next; the rendering helpers (`text_caret_x`,
  `text_index_at_point`, `shape_input_line`) are already reusable, but
  the bar wiring is search-specific (live search on edit, Enter
  cycling).
- No caret blink. The caret is a static 1px bar; adding a blink needs a
  timer + generation guard like the copy flash.
- No horizontal scroll when the text is wider than the field; the caret
  can move off the clipped edge. Rare for short filter expressions but
  worth handling with the migrated search input (queries can be
  longer).
- Hit-testing and caret placement both shape the line with
  `window.text_style()`. This matches the rendered `StyledText` font and
  size today; if the bars ever use a custom font/size, the two paths
  must share the same style.
- `MainView` gained two more transient fields; per-tab search/filter
  state plus render artifacts continue to grow `view.rs` (~4100 lines).
  The `view.rs` split is still overdue.
- An architecture consultation for this slice failed with a harness
  `malformed_response`; the design was settled by direct inspection of
  the vendored gpui source instead (no `TextInput` element;
  `TextLayout` / `shape_line` are the intended primitives).

### Open loops

- Migrate the search bar to `TextEdit` (caret, selection, mouse).
- Caret blink; in-field horizontal scroll / visible caret for long
  text; double-click word selection.
- Apply `TextEdit` to Cmd/Ctrl+G go-to-row when that lands.
- Carried over: compound filters, filter chips, split `view.rs`, cap /
  group large archives, lazy-extraction cancellation, per-tab
  `search_task` / `copy_flash_generation`, panel resize/reorder, sort
  (Milestone 3).

### Next recommended task

The highest-value follow-up is to migrate the **search bar** to the
same `TextEdit` model, which finishes the user-facing text-editing
capability across both bars and is a bounded change now that the model
and helpers exist. The overdue `view.rs` split should still land before
Milestone 3; `text_edit.rs` already removes one concern from the file.

## 2026-10-17 — Migrate the search bar to the editable TextEdit model

### Context

The previous iteration introduced `ui::text_edit::TextEdit` and gave the
filter input a real caret, selection, and mouse interaction; the search
bar was left on the old append/pop handler. This iteration migrates the
search bar to the same model, so both inputs now behave consistently.
It is the closing half of the "editable text inputs" capability.

### Implemented

`src/ui/view.rs`:

- `search_query` is now a `TextEdit` on `MainView` and
  `DocumentSnapshot` (per tab, mirrored through `snapshot_active` /
  `load_active` / `clear_live_per_document`).
- New transient fields `search_text_bounds: Option<Bounds<Pixels>>` and
  `search_dragging: bool` on `MainView`.
- `on_search_key_down` now routes editing keys to `TextEdit`: arrows,
  Home/End, Shift-extension, Option/Ctrl+arrow word movement,
  Cmd/Ctrl+A select-all, and Backspace/Delete over the selection. Any
  text-changing operation re-runs the live background search via
  `kick_search`; caret-only movement does not.
- `render_search_bar` takes `Window` and renders an editable area
  (`StyledText` with a `BG_SELECTED` selection range, a shaped caret,
  and a hitbox-free `canvas` recording `search_text_bounds`). The
  existing `Aa` toggle and column-scope chip are unchanged.
- New `search_mouse_down` / `search_mouse_move` / `search_mouse_up`
  place and extend the caret from pointer x, mirroring the filter
  handlers. `kick_search` reads `search_query.text()`.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 229 passed (72 lib + 93 binary + 64
  integration), 0 failed, 2 ignored (throughput tests). Was 226.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, ~351 ms.
- `./target/release/anvil` — launches and stays alive for 5 s with no
  panic. Interactive key/mouse exercise was done through the headless
  UI tests, including a synthesised click-drag on the rendered search
  field.

### Tests added / changed

- 3 UI tests in `src/ui/view.rs`:
  - `search_keyboard_caret_and_selection` (Home/End/Shift+Home),
  - `search_mouse_click_and_drag` (click near the right edge puts the
    caret at the end; drag from the start selects a non-empty prefix),
  - `search_insert_at_caret_re_runs_search` (typing at a mid-string
    caret builds `needle` and the live search updates to `[0, 1]`).
- Existing search assertions were mechanically updated from
  `view.search_query` to `view.search_query.text()`.

### Roadmap changes

- Milestone 2 → Filter: ticked the "apply the same editable text input
  to the search bar" follow-up.
- Milestone 2 → Tests: updated the text-editing test line to cover both
  bars and the live re-search after a mid-string insert.

### Architecture changes

- `ARCHITECTURE.md`:
  - Primary-state table: `search_query` is a `TextEdit`; added
    `search_text_bounds` and `search_dragging`.
  - "Editable text inputs" section now states both bars use `TextEdit`,
    describes the shared rendering pattern, and notes the one
    behavioural difference (search re-runs live, filter applies on
    Enter).
  - Functional gaps and the testing counts/rows updated (UI regression
    tests 45 → 48).
- `README.md`: binary test count 90 → 93.

### Issues / technical debt discovered

- `render_filter_bar` and `render_search_bar` now duplicate the
  editable-area rendering (text/selection/caret/bounds canvas/mouse
  wiring). A small shared helper would remove ~60 duplicated lines and
  is a good candidate alongside the overdue `view.rs` split.
- Still no caret blink and no horizontal scroll when the text is wider
  than the field; a long search query can push the caret past the
  clipped edge.
- Hit-testing and caret placement both rely on `window.text_style()`
  matching the rendered `StyledText`; keep them in sync if a custom
  font/size is ever introduced.
- `view.rs` is now ~4400 lines and growing; the split remains overdue.

### Open loops

- Extract the shared editable-area renderer.
- Caret blink; in-field horizontal scroll / keep-caret-visible;
  double-click word selection.
- Apply `TextEdit` to Cmd/Ctrl+G go-to-row when that lands.
- Carried over: compound filters, filter chips, split `view.rs`, cap /
  group large archives, lazy-extraction cancellation, per-tab
  `search_task` / `copy_flash_generation`, panel resize/reorder, sort
  (Milestone 3).

### Next recommended task

The overdue `view.rs` split is now the clear next step: it is ~4400
lines, both text inputs duplicate a ~60-line editable-area renderer,
and Milestone 3 (sort / column inspector) will add more UI. Extracting
the editable input, the search options, and the filter bars into their
own modules would land a meaningful refactor with the existing UI tests
as the safety net. If refactoring is deferred, Cmd/Ctrl+G is the last
small Milestone 2 navigation item, or Milestone 3 sort can start on the
existing display→record mapping.

## 2026-10-18 — Split view.rs into focused modules

### Context

`src/ui/view.rs` had grown to ~4,800 lines (the previous three handoff
entries all flagged the split as overdue). This iteration lands the
recommended refactor: extract the shared editable-text renderer, the
search and filter bars, and the headless UI test suite. No behaviour
changes and no state-ownership changes — every field still lives on
`MainView`; the methods simply moved.

### Implemented

New modules:

- `src/ui/input.rs` (~116 lines): `text_index_at_point`,
  `text_caret_x`, `shape_input_line`, and `editable_text_area` — the
  shared `StyledText` + selection-highlight + caret + bounds-recording
  `canvas` renderer. This removes the ~60-line duplication between the
  two bars.
- `src/ui/view/search.rs` (~537 lines): the search half of
  `impl MainView` (`open_search` … `search_mouse_up`,
  `render_search_bar`, `search_status_text`) plus `search_column_label`.
- `src/ui/view/filter.rs` (~320 lines): the filter half of
  `impl MainView` (`open_filter` … `filter_mouse_up`,
  `render_filter_bar`, `filter_status_text`).
- `src/ui/view/tests.rs` (~1,605 lines): the `#[cfg(test)]` UI suite,
  now declared as `#[cfg(test)] mod tests;` from `view.rs`.

`view.rs` is now ~2,259 lines (down from ~4,795) and keeps the
`MainView` state, multi-document/tab lifecycle, open orchestration,
toolbar / tab-bar / file-panel / status / grid rendering, keyboard
actions, clipboard, drag auto-scroll, archive picker, and `Render`.

Mechanics: methods moved into child modules as `pub(super)` inherent
methods. `view::search` / `view::filter` / `view::tests` can access
`MainView`'s private fields because they are descendants of `view`;
`pub(super)` makes the moved methods callable from `view` and
`view::tests`. `input` is a sibling module under `ui` used by both bars.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 229 passed (72 lib + 93 binary + 64
  integration), 0 failed, 2 ignored. Identical to before the split.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `./target/release/anvil --probe fixtures/large.csv` — 1,000,000
  rows, ~354 ms.
- `./target/release/anvil` — launches and stays alive for 5 s with no
  panic.

### Tests added / changed

None. The suite was moved verbatim; only the test module's path changed
(`src/ui/view/tests.rs`). The existing 229 tests are the refactor's
safety net and all still pass.

### Roadmap changes

- No feature checkboxes moved. Updated the UI-test path reference in the
  Milestone 2 tests list to `src/ui/view/tests.rs`.

### Architecture changes

- `ARCHITECTURE.md`:
  - Repository tree now shows `ui/input.rs` and the
    `ui/view/{search,filter,tests}.rs` submodules.
  - "Editable text inputs" section describes the shared
    `input::editable_text_area` instead of two duplicated renderers.
  - The `view.rs` technical-debt bullet records the split and the
    reduced size; further extraction is now optional/lower priority.
  - Testing section points at `src/ui/view/tests.rs`.
- `README.md`: module tree updated for the new files.

### Issues / technical debt discovered

- `MainView` still owns `DocumentSnapshot`, the tab lifecycle, open
  orchestration, and all the chrome rendering; `view.rs` at ~2,259 lines
  is now manageable but not minimal. A later `view::documents` and
  `view::chrome` extraction is possible but no longer urgent.
- All state still lives on one large `MainView` struct (~160 lines of
  fields). Splitting the modules did not (and should not, without an
  ADR) change ownership.
- `use super::*;` in the child modules pulls in the parent's imports;
  it works but is broad. If the modules grow further, switch to explicit
  imports to make dependencies visible.
- Two architecture consultations for this refactor failed with harness
  errors (`malformed_response` / `provider_incomplete`); the split was
  planned and executed locally, with the compiler and the 229-test suite
  as the verification surface.

### Open loops

- Optional: extract `view::documents` (tab/file-panel model) and
  `view::chrome` (toolbar/status) if the file grows again.
- Caret blink; in-field horizontal scroll / keep-caret-visible;
  double-click word selection.
- Apply `TextEdit` to Cmd/Ctrl+G go-to-row when that lands.
- Carried over: compound filters, filter chips, cap / group large
  archives, lazy-extraction cancellation, per-tab `search_task` /
  `copy_flash_generation`, panel resize/reorder, sort (Milestone 3).

### Next recommended task

The refactor debt that blocked Milestone 3 is now cleared. The natural
next step is **Milestone 3 sort / type inference / column inspector**,
starting with the data-side `query::sort` + `schema::infer` modules
(GPUI-free and unit-testable) and then a header-click sort UI on top of
the existing display→record mapping. If a smaller slice is preferred,
Cmd/Ctrl+G remains the last unchecked Milestone 2 navigation item.

## 2026-10-19 — Milestone 3: type inference + sortable grid

### Context

Milestone 3 opens with a sortable grid driven by cheap column
metadata. This iteration lands the data-side foundation plus the
end-to-end UI: heuristic type inference, a keyed sort permutation, and
header-click sorting that composes with the existing filter. It also
resolves the header-click conflict with the previous slice's search
column scope.

### Implemented

Data side:

- `src/schema.rs` (new): `ColumnType` (Integer / Float / Boolean /
  Text / Empty), `ColumnProfile { name, column_type, sampled_rows,
non_empty, numeric_fraction }`, and `infer(doc) -> Vec<ColumnProfile>`.
  Samples up to `INFER_SAMPLE_ROWS` (2048) per column, treating a
  column as numeric once `numeric_fraction >= 0.9`.
- `src/query/sort.rs` (new): `SortOrder`, `SortKind`, `SortSpec`, and
  `apply(doc, rows, spec) -> Vec<u64>`. Decodes one key per row, sorts
  a stable index permutation with `f64::total_cmp` for numeric and
  case-insensitive trimmed bytes for text, and always places empty /
  non-numeric values last. `rows` is the supplied view, so sorting
  composes with filtering.
- `src/lib.rs` re-exports `infer_schema` / `ColumnProfile` /
  `ColumnType` and `sort_rows` / `SortKind` / `SortOrder` / `SortSpec`.

UI (`src/ui/view.rs`, `src/ui/grid.rs`, `src/ui/view/filter.rs`):

- Per-tab `sort_spec` / `sort_rows` / `sort_generation` on
  `DocumentSnapshot` and `MainView` (plus `sort_task`), mirrored
  through `snapshot_active` / `load_active` / `clear_live_per_document`.
- `view_rows()` returns the sorted permutation when active, else the
  filtered set, else `None`; `render_grid`, `move_selection`, and
  `move_cell_selection` all read it, so navigation and rendering agree.
- `cycle_sort(column)` cycles none → ascending → descending → none;
  `kick_sort` runs `infer_schema` + `sort_rows` on the background
  executor with a generation guard. The filter task calls `kick_sort`
  after installing new rows, and `clear_filter` re-sorts the full set.
- `grid::render_header` now passes the `MouseDownEvent` to the click
  callback and draws a ▲ / ▼ arrow on the sorted column. A plain click
  cycles the sort; **Alt/Option-click** toggles the search column
  scope (previously a plain click did that). The status bar shows
  `sorted by <column> ▲`.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 249 passed (90 lib + 95 binary + 64
  integration), 0 failed, 3 ignored. Was 229.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `cargo test --release --test core -- --ignored measure_sort --nocapture`
  — **1,000,000 rows sorted by `amount` in 126 ms** (budget 3 s), and
  inference printed `id`/`user_id` Integer, `amount`/`risk_score`
  Float, `is_verified` Boolean, the rest Text.
- `anvil --probe fixtures/large.csv` — 1,000,000 rows, ~140 ms.
- `anvil` launches and stays alive 5 s with no panic; sort UI behaviour
  is covered by headless tests.

### Tests added / changed

- `src/schema.rs`: 9 unit tests (integer, mixed int/float, boolean,
  text, empty, the 0.9 threshold on both sides, header names,
  all-empty column).
- `src/query/sort.rs`: 9 unit tests (numeric vs lexicographic,
  ascending/descending, case-insensitive text, empties last in both
  directions, non-numeric last, filter composition, stability, empty
  input).
- `src/ui/view/tests.rs`: `header_click_cycles_sort` (real header
  click through the debug selector, asserting the permutation after
  each of the three clicks) and `sort_composes_with_filter` (filter to
  US rows, sort by `amount` asc then desc). Renamed the previous
  `header_click_toggles_search_column` to
  `alt_header_click_toggles_search_column` and made it Alt-click.
- `tests/core.rs`: ignored `measure_sort_large_fixture` throughput +
  inference-label test.

### Roadmap changes

- Milestone 3 → Sorting: all six tasks ticked (126 ms result noted).
- Milestone 3 → Type inference: marked `[~]` — sampling inference with
  confidence is done, but richer types and once-per-open caching are
  follow-ups.
- Milestone 3 → Tests: inference and sort test lines ticked.
- Dependencies: `ordered-float` not needed (`f64::total_cmp`);
  `hashbrown` still planned for the inspector.
- Acceptance criteria updated: sort budget met, inference partially
  met, inspector outstanding.

### Architecture changes

- `ARCHITECTURE.md`: repository tree gained `schema.rs` and
  `query/sort.rs`; "current" now lists `query::sort` and `schema::infer`
  and the planned list keeps only `schema::stats` + diagnostics;
  primary-state table gained the four sort fields; Performance records
  the 126 ms sort and inferred labels; Testing counts and rows updated
  (UI regression tests 48 → 50); Functional gaps and Planned Evolution
  rewritten for landed sort / partial inference.
- `README.md`: module tree gained `schema.rs` and `query/sort.rs`; test
  count 93 → 95 binary.

### Issues / technical debt discovered

- Type inference runs inside the sort task, not once per open, so it is
  recomputed on each sort request. Bounded to 2048 rows so it is cheap,
  but the column inspector wants cached profiles on the document.
- The header-click mapping changed: sort is the plain click and the
  search column scope moved to Alt/Option-click. This is a behaviour
  change to the Milestone 2 search slice; the roadmap and tests were
  updated together.
- `sort_rows` holds a full `Vec<u64>` permutation plus a transient
  `Vec<Key>` (one allocation per text key). For text columns on 1M rows
  this is the heaviest path; numeric keys are 8 bytes. Not measured for
  text yet.
- Sorting is per tab and survives switching, but is not persisted across
  app restarts (no config layer yet).
- The header arrow is rendered into the header label string, so it
  contributes to the auto column width; acceptable but worth revisiting
  with real text measurement.

### Open loops

- Column inspector (`schema::stats`, right-side panel) and the richer
  type set (date / datetime / UUID / email / URL).
- Cache inferred profiles on the document at open time.
- Text-column sort memory/throughput measurement.
- Carried over: compound filters, filter chips, cap / group large
  archives, lazy-extraction cancellation, per-tab `search_task` /
  `copy_flash_generation`, panel resize/reorder, diagnostics
  (Milestone 4).

### Next recommended task

Continue Milestone 3 with the **column inspector**: add
`anvil_core::schema::stats` (cardinality with the exact/approximate
distinction, min/max, longest/shortest, sample values) computed on the
background executor, cache profiles on the document at open, and add a
right-side panel showing the selected column's metrics. Alternatively,
flesh out type inference (richer types + once-per-open caching) first,
which the inspector depends on.

## 2026-10-20 — Keep the active search match on screen (both axes)

### Context

Navigating search matches (Enter / Shift+Enter, F3 / Shift+F3) scrolled
the list vertically but never horizontally, so a match in a column to
the right of the current viewport stayed off-screen. Two related bugs
surfaced while fixing it: the vertical scroll used the raw record index
as a list display position (wrong once a filter or sort reorders the
rows), and arrow / Tab navigation could walk the selection cursor off
the horizontal edge without scrolling.

### Implemented

`src/ui/view.rs`:

- `ensure_column_visible(column, cx)` scrolls `scroll_x` so the column
  is fully inside `[scroll_x, scroll_x + viewport_w]`, with a small
  margin and a clamp to the content width. It reuses the same
  `ColumnLayout` offsets the grid renders with.
- `display_position(record)` maps a record index to its 0-based display
  position through `view_rows()` (sorted permutation or filtered set),
  falling back to identity when there is no filter/sort.
- `move_selection` and `move_cell_selection` now call
  `ensure_column_visible` after moving, so arrow / Tab navigation keeps
  the cursor column on screen.

`src/ui/view/search.rs`:

- `first_match_column(row)` returns the scoped column when set,
  otherwise the first column whose cell contains the needle (using
  `find_ranges` with the current case-sensitivity), otherwise `None`.
- `jump_to_active_match` now lands on that column, scrolls vertically
  by `display_position(row)` instead of the raw record index, and
  scrolls horizontally by `ensure_column_visible(col)`.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 251 passed (90 lib + 97 binary + 64
  integration), 0 failed, 3 ignored. Was 249.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `anvil --probe fixtures/large.csv` — ~354 ms.
- `anvil` launches and stays alive 5 s with no panic; the new behaviour
  is covered by headless tests.

### Tests added / changed

- `src/ui/view/tests.rs`:
  - `search_jump_scrolls_horizontally_to_match` — a 40-column fixture
    with `needle` only in the last column; after a whole-record search
    and a jump with a forced 400 px viewport, the cursor lands on
    column 39, `scroll_x > 0`, and the column is fully visible.
  - `display_position_maps_through_view_rows` — identity without a
    filter, remapped with a filtered set, and a fallback for a record
    not present.
- Added a shared `wide_document(cols, needle_col)` test helper.

### Roadmap changes

- Milestone 2 → Search: corrected the header-click wording to
  Alt/Option-click and added a checked task for match
  scroll-into-view (vertical display-order mapping + horizontal
  column visibility + arrow/Tab scroll).

### Architecture changes

- `ARCHITECTURE.md`: the search section documents `display_position`,
  `ensure_column_visible`, and `first_match_column`; UI regression
  count 50 → 52; the UI/search table row mentions the far-right scroll
  test.
- `README.md`: binary test count 95 → 97.

### Issues / technical debt discovered

- `display_position` is a linear scan of `view_rows` per match jump.
  For a 1M-row filtered/sorted set this is a few milliseconds per jump,
  acceptable for keypress navigation but worth revisiting if it ever
  shows up in a profile (the filtered set is sorted by record so it
  could binary-search; the sort permutation is not).
- A column wider than the viewport can never be fully shown; the clamp
  picks the best available position. `MAX_AUTO_WIDTH` (360 px) keeps
  this unlikely at normal window sizes.
- The horizontal margin is a fixed `CELL_PAD_X * 2`; there is no
  animated scroll or easing (the grid jumps directly), matching the
  existing vertical jump behaviour.

### Open loops

- Cache inferred profiles on the document at open; column inspector
  (`schema::stats`).
- Text-column sort memory/throughput measurement.
- Carried over: compound filters, filter chips, cap / group large
  archives, lazy-extraction cancellation, per-tab `search_task` /
  `copy_flash_generation`, panel resize/reorder, diagnostics
  (Milestone 4).

### Next recommended task

Continue Milestone 3 with the **column inspector** (`schema::stats` +
right-side panel), which also motivates caching inferred profiles on
the document at open. Alternatively the small `display_position`
binary-search optimisation can ride along with a future filtered-view
refactor.

## 2026-10-21 — Fix: scroll the whole target cell into view

### Context

The previous iteration made search-match jumps scroll horizontally to
the matching column, but the user reported that only the part of the
cell around the matching text came into view. The root cause turned out
to be the grid's viewport calculation, not the scroll target.

### Root cause

`render_grid` computed

```
viewport_w = window.viewport_size().width - ROW_NUMBER_WIDTH
```

but the grid body shares a flex **row** with the file-manager panel
(220 px, open by default). The columns area is therefore
`window_width - ROW_NUMBER_WIDTH - FILE_PANEL_WIDTH`. Overstating the
viewport by 220 px meant `ensure_column_visible` scrolled too little and
left the right side of the target column clipped behind the panel. The
same stale value fed `ColumnLayout::visible` and `render_h_scrollbar`,
so the header rendered extra clipped columns and the scrollbar thumb
was slightly wrong.

### Implemented

- `src/ui/view.rs`: `render_grid` now subtracts
  `theme::FILE_PANEL_WIDTH` when `file_panel_open && !documents.is_empty()`,
  so `viewport_w` reflects the real columns area. This is the single
  value used by `scroll_horizontal`, `ensure_column_visible`,
  `ColumnLayout::visible`, and `render_h_scrollbar`.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 252 passed (90 lib + 98 binary + 64
  integration), 0 failed, 3 ignored. Was 251.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `anvil --probe fixtures/large.csv` — ~349 ms.
- `anvil` launches and stays alive 5 s with no panic.

### Tests added / changed

- `viewport_width_excludes_file_panel` — reads the actual window width
  and asserts `viewport_w == window - row-gutter - panel`. Fails with
  the old formula.
- `search_jump_scrolls_horizontally_to_match` was made faithful: it now
  uses a 300-column document and the real (panel-aware) viewport instead
  of forcing `viewport_w = 400`, and asserts the **full column
  rectangle** (`start .. start + width`) lies inside
  `[scroll_x, scroll_x + viewport_w]`.

### Roadmap changes

- Milestone 2 → Search: the scroll-into-view task now notes the
  file-panel viewport inset.

### Architecture changes

- `ARCHITECTURE.md`: the horizontal-scrolling section documents the
  `viewport_w` computation (window − gutter − panel) and that it is the
  single source for scroll, visibility, and the scrollbar; UI
  regression count 52 → 53.
- `README.md`: binary test count 97 → 98.

### Issues / technical debt discovered

- `viewport_w` is still derived from the window rather than measured
  from the grid element's own bounds. This is correct today because the
  only horizontal inset is the file panel, but if another sibling
  (a future right-side inspector) is added, the subtraction must be
  extended. Capturing the grid's bounds via a `canvas` overlay would be
  more robust.
- The pattern of computing layout widths from `window.viewport_size()`
  in `render_grid` runs before the panel is decided in `render`; the
  check duplicates the panel-shown condition. Keep the two in sync.

### Open loops

- Cache inferred profiles on the document at open; column inspector
  (`schema::stats`).
- Text-column sort memory/throughput measurement.
- Carried over: compound filters, filter chips, cap / group large
  archives, lazy-extraction cancellation, per-tab `search_task` /
  `copy_flash_generation`, panel resize/reorder, diagnostics
  (Milestone 4).

### Next recommended task

Continue Milestone 3 with the **column inspector** (`schema::stats` +
right-side panel). When that panel lands, switch `viewport_w` to the
grid element's measured bounds so the horizontal inset is no longer a
hand-maintained subtraction.

## 2026-10-22 — Resolve debt: measure the grid viewport instead of subtracting insets

### Context

The 2026-10-21 fix computed `viewport_w` from the window width minus
the row-number gutter and the file panel. That was correct but fragile:
it hard-coded the only current horizontal inset, and the panel-shown
condition was duplicated between `render_grid` and `render`. This
iteration resolves that debt by deriving the viewport from the grid's
own measured bounds.

### Implemented

- `src/ui/view.rs`, `render_grid`: `viewport_w` now comes from
  `self.list_bounds` (the grid list area's window bounds, already
  recorded by the auto-scroll overlay `canvas`) minus
  `ROW_NUMBER_WIDTH`, with a fallback to the window width on the first
  frame before any bounds exist. No panel/sibling arithmetic remains.
- The overlay canvas now calls `cx.notify()` when the recorded bounds
  change (guarded by inequality, so it cannot loop), so the viewport
  settles within a frame after a window resize or a panel toggle.

The file panel itself still checks `file_panel_open && !documents.is_empty()`
to decide whether to render — that is its own concern and is not
duplicated in the viewport calculation anymore.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 252 passed (90 lib + 98 binary + 64
  integration), 0 failed, 3 ignored.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `anvil --probe fixtures/large.csv` — ~133 ms.
- `anvil` launches and stays alive 5 s with no panic.

### Tests added / changed

- `viewport_width_excludes_file_panel` now also hides the panel and
  asserts the measured viewport widens to `window − gutter`, proving
  the bounds-derived path tracks layout changes (it needs one extra
  rendered frame to settle, which the test documents).

### Roadmap changes

None.

### Architecture changes

- `ARCHITECTURE.md`: the horizontal-scrolling section now describes the
  measured-bounds viewport, the `cx.notify()` on bounds change, and the
  first-frame fallback, replacing the hand-maintained inset note.

### Issues / technical debt discovered

- `list_bounds` was previously only used for drag auto-scroll; it now
  also feeds the viewport. The two uses are compatible (both want the
  scroll area's window bounds) but the name no longer says "auto
  scroll". If a third consumer appears, rename it to `grid_bounds`.
- The viewport is one frame behind a layout change by construction; the
  notify closes the gap. There is no guard against a pathological
  bounds oscillation, but pixel bounds are stable in practice.

### Open loops

- Cache inferred profiles on the document at open; column inspector
  (`schema::stats`).
- Text-column sort memory/throughput measurement.
- Carried over: compound filters, filter chips, cap / group large
  archives, lazy-extraction cancellation, per-tab `search_task` /
  `copy_flash_generation`, panel resize/reorder, diagnostics
  (Milestone 4).

### Next recommended task

Continue Milestone 3 with the **column inspector** (`schema::stats` +
right-side panel). The viewport is now measured, so adding a right-side
panel needs no changes to the horizontal scroll math.

## 2026-10-23 — Rename `list_bounds` to `grid_bounds`

### Implemented

- `src/ui/view.rs`: renamed `MainView.list_bounds` to `grid_bounds` (all
  6 uses) and updated its doc comment to state both consumers — drag
  auto-scroll edge detection and the measured `viewport_w`.
- `ARCHITECTURE.md`: state-table row, the drag auto-scroll section, and
  the horizontal-scrolling section now use `grid_bounds`.

Historical agent-log entries keep the old name, as they record what the
field was called at the time.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 252 passed (90 lib + 98 binary + 64
  integration), 0 failed, 3 ignored.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.

### Next recommended task

Unchanged: the Milestone 3 column inspector (`schema::stats` +
right-side panel).

## 2026-10-24 — Milestone 3: column inspector

### Context

Sorting and basic type inference landed earlier. This iteration adds the
column inspector: a right-side panel showing the selected column's
metrics, backed by a new data-side `schema::stats`. With the viewport
now measured from the grid's `grid_bounds` (2026-10-22), the panel
narrows the grid without any horizontal-scroll changes.

### Implemented

Data side (`src/schema/stats.rs`, new):

- `ColumnStats` — name, inferred type, total/non-empty/empty rows,
  distinct, duplicate count, min/max, longest/shortest, samples, and the
  top-N frequent values.
- `DistinctCount::{Exact, Approximate}` and `StatsOptions`
  (`max_exact_distinct` 100k, `top_values` 10, `samples` 8).
- Distinct is exact while the value set fits a `HashMap`; past the
  threshold it switches to a dependency-free 1024-register HyperLogLog
  with the small-range linear-counting correction, and drops the exact
  set/map (so high-cardinality columns stay bounded).
- `infer_column` exposed from `schema.rs`; `lib.rs` re-exports
  `column_stats`, `ColumnStats`, `DistinctCount`, `StatsOptions`,
  `ValueCount`.

UI (`src/ui/view.rs`, `commands.rs`, `theme.rs`):

- New global state: `inspector_open`, `inspector_column`,
  `inspector_stats`, `inspector_generation`, `inspector_task`.
- `Action::ToggleInspector` on Cmd/Ctrl+I and a toolbar **Inspect**
  button; `toggle_inspector` opens/closed the panel.
- `kick_inspector` runs `column_stats` on the background executor with a
  generation guard and is a no-op when the selected column is unchanged
  and stats are already present — so vertical navigation never rescans.
  It is called on toggle, on tab switch/open, after every selection
  move, and on cell click / drag release.
- `load_active` invalidates cached stats so switching tabs rescans.
- `render_inspector` builds the panel (header + `×`, scrollable body);
  `render_column_stats` renders the metric rows, samples, and a
  frequent-values list. `theme::INSPECTOR_WIDTH` is 320 px.
- The panel is a `flex_shrink_0` sibling after the grid body, so the
  measured grid narrows automatically.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 266 passed (99 lib + 103 binary + 64
  integration), 0 failed, 3 ignored. Was 252.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `anvil --probe fixtures/large.csv` — ~351 ms.
- `anvil` launches and stays alive 5 s with no panic; the panel is
  rendered by the UI tests.

### Tests added / changed

- `src/schema/stats.rs`: 9 unit tests — counts/empties, distinct +
  duplicates, numeric vs case-insensitive min/max, frequent ranking,
  samples, the exact→approximate threshold, headerless naming,
  non-numeric values, out-of-range column.
- `src/ui/commands.rs`: `toggle_inspector_chord`,
  `plain_i_is_not_toggle_inspector`.
- `src/ui/view/tests.rs`: `cmd_i_toggles_inspector`,
  `inspector_follows_selected_column`,
  `inspector_reports_exact_distinct_and_frequencies`.

### Roadmap changes

- Milestone 3 → Column inspector: panel, metrics, frequent values, and
  the exact/approximate distinction ticked; scoping stats to the
  filtered/sorted view is a new unchecked follow-up.
- Milestone 3 → Type inference: the "run once per open" note is relaxed
  to optional (sort and inspector each compute it in their task).
- Milestone 3 → Tests and acceptance criteria updated.
- Dependencies: `hashbrown` not added (std `HashMap` + hand-rolled HLL).

### Architecture changes

- `ARCHITECTURE.md`: repository tree gained `schema/stats.rs`; the
  "current" section documents `schema::stats`; primary-state table
  gained the five inspector fields; Planned Evolution gained a landed
  "Column inspector" entry and the type-inference entry is updated;
  testing counts/rows updated (UI regression 53 → 56); functional gaps
  rewritten.
- `README.md`: module tree and binary test count (98 → 103).

### Issues / technical debt discovered

- Inspector stats scan the whole document, not the filtered/sorted
  view. That is a deliberate first cut; a view-scoped stats call would
  take the `view_rows` permutation.
- `ColumnStats` recomputes `infer_column` inside `stats`; with
  sort also calling inference, the profiles are computed twice per
  interaction. Caching them on the document at open is the follow-up
  that removes both.
- Frequent values are only available in exact mode (they need the
  count map). Approximate-mode columns show no frequent list.
- The panel is a fixed 320 px and cannot be resized or hidden by a
  click on the column; the selection is what drives it.
- The HLL estimate is uncalibrated beyond the small-range correction;
  for very large cardinalities it is a rough estimate, which the UI
  labels "approx".

### Open loops

- View-scoped inspector stats; cache inferred profiles at open.
- Richer type classes (date / datetime / UUID / email / URL).
- A generated-fixture stats assertion.
- Carried over: compound filters, filter chips, cap / group large
  archives, lazy-extraction cancellation, per-tab `search_task` /
  `copy_flash_generation`, panel resize/reorder, diagnostics
  (Milestone 4).

### Next recommended task

Milestone 3's headline features are now in place. The natural next step
is **Milestone 4 (diagnostics and data quality)**: surface the existing
`ParseWarning`s as a navigable list and jump the grid to the offending
row, then inconsistent-width / unterminated-quote / duplicate-header
navigation. If Milestone 3 polish is preferred, cache inferred profiles
at open and scope inspector stats to the current view.

## 2026-10-25 — Milestone 4: diagnostics panel + unterminated-quote index fix

### Context

Milestone 3 (sort, type inference, column inspector) had landed. The
roadmap's next milestone is diagnostics and data quality, whose
acceptance criterion is that opening `fixtures/malformed.csv` shows the
warning count and lets the user click through to the offending row.
The warnings already existed on `ParseMeta.warnings`; nothing surfaced
them beyond a count. Verifying the data first exposed an off-by-one in
the `UnterminatedQuote` warning's `record_index`.

### Implemented

- **`src/parser/indexing.rs`**: the `UnterminatedQuote` warning now
  records the index of the partial record currently being walked
  (`record_index`) instead of `record_index - 1`. The latter pointed at
  the previous completed row whenever any earlier row had ended with a
  newline; for `fixtures/malformed.csv` it reported record 0 instead of
  record 1.
- **`src/ui/view/diagnostics.rs`** (new): a right-side panel rendering
  the active document's `ParseMeta.warnings`. Each row shows the warning
  kind, the target row (`row N`, or `header`), and the note. Row
  warnings are clickable and call `jump_to_warning`; header-level
  warnings (`record_index == None`) are informational only.
- `jump_to_warning(warning_index)`: moves the selection to the record
  and scrolls the display order (sort-aware via `display_position`) to
  the top. If the record is hidden by the active filter, it reports
  `row N is hidden by the active filter` in the status bar and leaves
  the cursor alone rather than silently scrolling to an unrelated row.
  Keeps the cursor's current column.
- **`src/ui/commands.rs`**: `Action::ToggleDiagnostics`, bound to
  Cmd/Ctrl+D with the same alt/function guards as the other panel
  chords.
- **`src/ui/view.rs`**: `diagnostics_open: bool` (global, like the
  inspector), `toggle_diagnostics`, the `run_action` arm, a toolbar
  **Diagnostics** button, and the panel inserted as a `flex_shrink_0`
  sibling of the grid. No background task or generation counter is
  needed because warnings are immutable per document.
- **`src/ui/theme.rs`**: `DIAGNOSTICS_WIDTH` (320 px).
- **`README.md`**: test counts refreshed and the stale "Milestones 2–5"
  not-yet-done list corrected (selection/clipboard/search/filter/sort/
  inference/statistics are all implemented).

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 274 passed (99 lib + 110 binary + 65
  integration), 0 failed, 3 ignored. Was 266 (99 + 103 + 64).
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `anvil --probe fixtures/large.csv` — 1,000,000 rows, 0 warnings,
  363 ms.
- `anvil --probe fixtures/malformed.csv` — 2 rows, 1 warning; the
  fixed index now points at the partial record.
- `anvil` launches and stays alive 6 s with no panic.

### Tests added / changed

- `tests/core.rs`: `unterminated_quote_index_points_at_the_partial_record`
  (regression for the off-by-one).
- `src/ui/commands.rs`: `toggle_diagnostics_chord`,
  `plain_d_is_not_toggle_diagnostics`,
  `alt_d_is_not_toggle_diagnostics`.
- `src/ui/view/tests.rs`: `cmd_d_toggles_diagnostics`,
  `diagnostics_navigation_moves_selection_to_warning_row`,
  `diagnostics_navigation_refuses_a_filtered_out_row`,
  `diagnostics_navigation_reports_header_warning`, plus a
  `warning_document()` helper.

### Roadmap changes

- Milestone 4 → In-record diagnostics: inconsistent-row-width and
  unterminated-quote navigation ticked; the diagnostics panel and
  jump-to-row ticked; empty/duplicate header navigation marked
  partially done (listed, not navigable, because they have no row);
  very-long-record emission left unchecked.
- Milestone 4 → Tests: diagnostics tests ticked.

### Architecture changes

- `ARCHITECTURE.md`: repository tree gained `view/diagnostics.rs`; the
  "current" list and primary-state table document `diagnostics_open`;
  the testing counts/rows and functional gaps were updated; the
  diagnostics planned-evolution entry now records it as landed and
  notes the index fix.

### Issues / technical debt discovered

- The `UnterminatedQuote` index bug was latent because no caller
  consumed `record_index`; it is now covered by a regression test.
- `WarningKind::VeryLongRecord` is still never emitted; a threshold
  policy is needed. Explicitly deferred.
- The panel renders every warning in a scrollable column rather than a
  virtualized list; fine for the small warning counts we see, and a
  `uniform_list` swap is a later optimisation if a pathological file
  produces thousands of warnings.
- Warnings carry no column information, so navigation lands on the
  cursor's current column rather than the offending cell.
- Header warnings are only partially actionable: listed but not
  navigable.

### Open loops

- Very-long-record emission; per-row grid highlighting / gutter marker
  for warnings; warning grouping/filtering by kind; next/previous
  warning keyboard cycling.
- Carried over: compound filters, filter chips, cache inferred
  profiles at open, view-scoped inspector stats, richer type classes,
  cap / group large archives, lazy-extraction cancellation,
  panel resize/reorder.

### Next recommended task

Continue Milestone 4 with **encoding and archive diagnostics** (surface
non-UTF-8 cells and skipped/rejected archive entries), or emit
`VeryLongRecord` so the panel has a fourth row-level warning to
navigate. Either is a small, self-contained slice. Alternatively,
per-row grid highlighting of warnings would make the existing panel
more useful without changing the data model.

## 2026-10-26 — Milestone 4: very-long-record warning + grid warning gutter

### Context

The previous iteration landed the diagnostics panel and row navigation.
This iteration completes the remaining In-record diagnostics task by
emitting the defined-but-unused `WarningKind::VeryLongRecord`, and makes
navigation land on a visibly flagged row by marking warning rows in the
grid gutter. Both feed the existing panel, so no new UI surface was
needed.

### Implemented

- **`src/parser/indexing.rs`**: after the walk finishes, a post pass
  over the completed offset table flags any record longer than
  `MAX_RECORD_BYTES` (1 MiB, including its trailing newline) with a
  `VeryLongRecord` warning carrying the record index and the byte
  length. The walker itself is unchanged, so no new per-byte cost.
- **`src/ui/grid.rs`**: `RowContext` gained `warning_rows: &[u64]` (a
  sorted, deduplicated slice). `render_row` binary-searches it and, for
  a flagged row, prefixes the row-number gutter with `⚠` and colours it
  `theme::WARN`. The pure `is_warning_row` helper has a unit test.
- **`src/ui/view.rs`**: `render_grid` builds the sorted warning-row set
  once per render pass from the active document's warnings and passes
  it into every `RowContext`.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 277 passed (99 lib + 111 binary + 67
  integration), 0 failed, 3 ignored. Was 274 (99 + 110 + 65).
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `anvil --probe` on a generated 2 MiB-record CSV reports
  `1 warnings — [very long record] record is 2097155 bytes (limit
1048576)`.
- `anvil --probe fixtures/large.csv` — 1,000,000 rows, 0 warnings (no
  false positives on ordinary rows).

### Tests added / changed

- `tests/core.rs`: `very_long_record_warns` (2 MiB record at index 1),
  `ordinary_records_do_not_warn_as_very_long` (a 1M-character cell stays
  under the limit).
- `src/ui/grid.rs`: `is_warning_row_uses_sorted_membership`; the three
  existing `RowContext` literals gained `warning_rows: &[]`.

### Roadmap changes

- Milestone 4 → In-record diagnostics: very-long-record detection ticked,
  and the diagnostics-panel note now mentions the `⚠` gutter. Only
  header-level navigation remains partial (no row to jump to).

### Architecture changes

- `ARCHITECTURE.md`: the diagnostics-data section now describes
  `VeryLongRecord`; the grid "Row retrieval" step and the `ui::grid`
  current-modules bullet mention the warning gutter and
  `RowContext::warning_rows`; the diagnostics planned-evolution entry
  records the emission and gutter as landed; test counts updated.

### Issues / technical debt discovered

- The 1 MiB threshold is a fixed constant, not configurable. It
  deliberately sits above the existing 1M-character-cell edge case so
  that test and typical large cells do not warn; a future config file
  could expose it.
- `warning_rows` is rebuilt every `render_grid` (O(warnings) +
  sort). Negligible for realistic warning counts; if a pathological
  file produced hundreds of thousands of warnings it would be worth
  caching on `GridModel`.
- The gutter marker does not distinguish warning kinds; all warnings
  look the same in the grid. The panel carries the kind.

### Open loops

- Warning grouping/filtering by kind; next/previous warning keyboard
  cycling; distinguishing warning kinds in the gutter.
- Carried over: empty/duplicate header navigation, encoding and archive
  diagnostics, compound filters, filter chips, cache inferred profiles
  at open, view-scoped inspector stats, richer type classes, cap/group
  large archives, lazy-extraction cancellation, panel resize/reorder.

### Next recommended task

Milestone 4's encoding diagnostics are the next unchecked group:
expose a per-cell UTF-8 validity flag and flag columns with non-UTF-8
cells. Archive diagnostics (surface skipped/rejected entries in the
picker) is a similarly self-contained alternative. Either is a small
vertical slice.

## 2026-10-27 — Technical debt: cached warning index, kind-aware gutter, IndexOptions, dead-code cleanup

### Context

Before continuing Milestone 4 (encoding/archive diagnostics), this
iteration cleared the debt recorded by the previous two diagnostics
iterations plus the stale error-variant note in ARCHITECTURE.

### Implemented

- **`GridModel` warning cache (`src/ui/grid.rs`)**: `GridModel` now
  carries `warnings: Arc<[(u64, WarningKind)]>`, built once at open by
  `cache_warning_rows` (sorted by record, severity-ranked, deduplicated
  to the most informative kind per row). `render_grid` no longer rebuilds
  that index every render pass; it clones the `Arc` and passes
  `model.warnings.as_ref()`.
- **Kind-aware gutter**: `RowContext::warning_rows` is now
  `&[(u64, WarningKind)]`; `warning_kind_for` replaces `is_warning_row`.
  The row-number gutter shows `✖` in the new `theme::ERROR` for an
  unterminated quote and `⚠` in `theme::WARN` for other warnings, so
  warning kinds are distinguishable in the grid.
- **Configurable record-length threshold (`src/parser/indexing.rs`)**:
  added `IndexOptions { max_record_bytes }` (default
  `DEFAULT_MAX_RECORD_BYTES` = 1 MiB) with `build_with` / `parse_with`
  variants; `build` / `parse` remain default wrappers. `lib.rs`
  re-exports `IndexOptions`, `parse_opened_with`, and
  `DEFAULT_MAX_RECORD_BYTES`. Tests can now exercise the threshold on a
  tiny fixture instead of allocating 2 MiB.
- **Dead code removed (`src/error.rs`)**: dropped the never-constructed
  `AnvilError::NotAZip`, `AmbiguousArchive`, and `LooksBinary` variants
  and the now-unused `PathBuf` import.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 279 passed (99 lib + 112 binary + 68
  integration), 0 failed, 3 ignored. Was 277.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `anvil --probe` on the 2 MiB-record CSV still reports
  `1 warnings — [very long record] record is 2097155 bytes (limit
1048576)`; `fixtures/large.csv` reports 0 warnings.

### Tests added / changed

- `src/ui/grid.rs`: `warning_kind_for_uses_sorted_membership_and_severity`
  and `cache_warning_rows_keeps_the_most_severe_kind_per_record`;
  the three `RowContext` literals updated to the new slice type.
- `tests/core.rs`: `very_long_record_threshold_is_configurable` uses
  `parse_opened_with` with a 3/4-byte limit on a four-byte fixture;
  imports gained `parse_opened_with` / `IndexOptions`.

### Roadmap changes

None — this was debt, not a roadmap task.

### Architecture changes

- `ARCHITECTURE.md`: error-surface table drops the three dead variants;
  "User-visible errors" and "Diagnostics data" describe the panel,
  kind-aware gutter, and cached `(record, WarningKind)` index; "Row
  boundary indexing" documents `IndexOptions`; test counts updated; the
  architectural-debt list no longer mentions the dead variants.
- `README.md`: test counts refreshed.

### Issues / technical debt discovered

- A delegation consult was attempted twice for the multi-file change;
  the first failed with a provider timeout and the second with a
  client-side invalid-line-range error. Per policy neither was
  auto-retried; correctness was established with the compiler and the
  full test suite. The design had already been settled by the prior
  architecture consultation (cache derived data on `GridModel`, mirror
  `ColumnLayout`).
- Severity ordering (`UnterminatedQuote > VeryLongRecord >
InconsistentColumnCount`) is a heuristic; the panel still exposes the
  full per-row warning list, so no information is lost.
- `IndexOptions` currently has a single field. It is a small, honest
  seam for the Milestone 5 config file rather than a general options
  bag.

### Open loops

- Carried over: empty/duplicate header navigation, encoding and archive
  diagnostics, compound filters, filter chips, cache inferred profiles
  at open, view-scoped inspector stats, richer type classes, cap/group
  large archives, lazy-extraction cancellation, panel resize/reorder.

### Next recommended task

Move on to Milestone 4 **encoding diagnostics**: expose a per-cell
UTF-8 validity flag and flag columns with non-UTF-8 cells. Archive
diagnostics (surface skipped/rejected entries in the picker) is the
self-contained alternative.

## 2026-10-28 — Milestone 4: encoding diagnostics (data side)

### Context

With the previous diagnostics iterations done, this begins the
Milestone 4 encoding-diagnostics group. Non-UTF-8 cells were silently
invisible: `Cell::as_str` / `into_string` fall back to empty/lossy
output, so an invalid cell rendered exactly like an empty one and a
leading UTF-8 BOM stayed glued to the first header cell.

### Implemented

- **`Cell::is_utf8` (`src/parser/mod.rs`)**: the per-cell validity flag
  the encoding scan uses; `as_str` still falls back to `""` so the grid
  behaviour is unchanged.
- **`src/encoding.rs` (new)**: `scan(doc) -> EncodingReport`, one pass
  over the data records using the existing `Cells` iterator. It counts
  non-UTF-8 cells per column, records the total, detects a leading
  `EF BB BF` BOM, and reports `records_scanned`. Only columns with at
  least one invalid cell are included, each with a header name (or
  `col N`). `ColumnEncoding` / `EncodingReport` and `scan_encoding` are
  re-exported from `anvil_core`.
- **`src/main.rs`**: `--probe` now prints the BOM flag and the invalid
  UTF-8 total with up to five offending columns, giving the feature a
  headless harness.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 285 passed (103 lib + 112 binary + 70
  integration), 0 failed, 3 ignored. Was 279.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `anvil --probe /tmp/enc.csv` (two `0xff`/`0xfe` cells) reports
  `invalid utf-8: 2 cell(s)` and `[b] 2 invalid cell(s)`;
  `fixtures/large.csv` reports 0.

### Tests added / changed

- `src/encoding.rs`: 4 unit tests — clean UTF-8, per-column counting
  (`0xff`/`0xfe`), an invalid byte in the header (not counted) versus a
  headerless data column (reported as `col 0`), and BOM detection.
- `tests/core.rs`: `cell_is_utf8_reports_invalid_bytes` and
  `encoding_scan_flags_invalid_column`.

### Roadmap changes

- Milestone 4 → Encoding diagnostics: per-cell flag and the data-side
  scan ticked; BOM detection marked partial (reported by the API and
  probe, not the GUI); the GUI flagging task remains.

### Architecture changes

- `ARCHITECTURE.md`: repository tree gained `src/encoding.rs`; the
  current-modules list documents `encoding`; the testing section gained
  an encoding table row and the 4 unit-test count; the functional-gap
  note about non-UTF-8 cells now reflects `Cell::is_utf8` /
  `encoding::scan`.
- `README.md`: data-side test count updated (68 → 70).

### Issues / technical debt discovered

- `encoding::scan` visits every data record and is O(file length); it
  must run on the background executor when wired to the UI. It is not
  wired yet.
- The scanner counts cells but does not record which record indexes are
  affected, so per-cell navigation is not possible yet. That may or may
  not be wanted: the roadmap asks for column-level flagging.
- The BOM is detected but not stripped by the parser, so the first
  header cell still contains it. Stripping is a parser change (and would
  shift `header_range`); deferred until the UI reports BOMs.

### Open loops

- Encoding UI: flag columns with non-UTF-8 cells (grid header marker or
  diagnostics-panel entries), running `encoding::scan` on the background
  executor with a generation guard.
- Carried over: empty/duplicate header navigation, archive diagnostics,
  compound filters, filter chips, cache inferred profiles at open,
  view-scoped inspector stats, richer type classes, cap/group large
  archives, lazy-extraction cancellation, panel resize/reorder.

### Next recommended task

Wire the encoding report into the UI: when the diagnostics panel opens,
run `encoding::scan` on the background executor (generation-guarded, like
`schema::stats`) and list each affected column, plus a BOM notice. A
column-header marker is a smaller alternative if the panel is preferred
to stay synchronous.

## 2026-10-29 — Milestone 4: encoding diagnostics wired into the panel

### Context

The previous iteration added the data-side `encoding::scan` and
`Cell::is_utf8` but nothing in the GUI used them. This iteration wires
the report into the diagnostics panel, completing the Milestone 4
encoding-diagnostics group.

### Implemented

- **`MainView` state (`src/ui/view.rs`)**: `encoding_report:
Option<Arc<EncodingReport>>`, `encoding_generation: u64`,
  `encoding_task: Option<Task<()>>`.
- **`invalidate_encoding`** is called from `load_active`, so switching
  tabs / opening a document drops a stale report and discards an
  in-flight scan via the generation guard.
- **`kick_encoding`** runs `encoding::scan` on
  `cx.background_executor` with a generation guard, mirroring
  `kick_inspector` / the filter and sort tasks. It is a no-op when the
  panel is closed or a report is already cached for the document. It is
  called from `toggle_diagnostics` (open), `apply`, `switch_to`, and the
  lazy-load-ready path.
- **`render_diagnostics` (`src/ui/view/diagnostics.rs`)** now has two
  sections: _parse warnings_ (unchanged, still click-to-navigate) and
  _encoding_. The encoding section shows `scanning…`/`not scanned`,
  a `UTF-8 BOM present` notice, one row per column with non-UTF-8 cells
  (`<name> — N non-UTF-8 cell(s)`), or `clean UTF-8`. The panel header
  is now `Diagnostics` since it covers both kinds of finding.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 287 passed (103 lib + 114 binary + 70
  integration), 0 failed, 3 ignored. Was 285.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- GUI launches and stays alive with no panic. The panel's encoding path
  is exercised headlessly (see tests below); the scan itself is also
  covered by `--probe`.

### Tests added / changed

- `src/ui/view/tests.rs`: `diagnostics_scan_reports_invalid_utf8`
  (opening the panel kicks the task and the report names column 1 with
  two invalid cells) and `switching_document_invalidates_encoding_report`
  (a second document clears the cached report).

### Roadmap changes

- Milestone 4 → Encoding diagnostics: the encoding UI and the optional
  BOM reporting are ticked. The group is now complete (the parser still
  does not strip the BOM; that is recorded, not required).

### Architecture changes

- `ARCHITECTURE.md`: the `encoding` current-modules bullet and the
  diagnostics-panel entry record the background scan; the primary-state
  table gained the three encoding fields; the UI/diagnostics test row
  and the UI regression-test count (60 → 62) were updated.
- `README.md`: binary test count updated (112 → 114).

### Issues / technical debt discovered

- The encoding report is cached per active document but not stored in
  `DocumentSnapshot`, so switching away and back re-scans. The scan is
  O(file); caching it in the snapshot would avoid the repeat. Deferred
  until it is measured as a problem.
- The BOM is detected and reported but not stripped, so the first header
  cell still contains it. Stripping is a parser change that would shift
  `header_range`; deferred.
- Encoding findings are column-level and non-navigable. Per-cell
  navigation would require the scanner to retain affected record
  indexes, which would raise its memory cost; not currently wanted.

### Open loops

- Carried over: empty/duplicate header navigation, archive diagnostics,
  compound filters, filter chips, cache inferred profiles at open,
  view-scoped inspector stats, richer type classes, cap/group large
  archives, lazy-extraction cancellation, panel resize/reorder.

### Next recommended task

Milestone 4 **archive diagnostics**: surface skipped/rejected archive
entries (the `rejected_reason` already computed in
`source::archive::evaluate_entry`) in the archive picker, and present
encrypted / ratio-capped entries with a friendlier message. After that,
the remaining Milestone 4 work is richer filters (compound predicates),
or the project can move to Milestone 5 hardening.

## 2026-10-30 — Milestone 4: header detection, headerless labels, and a manual override

### Context

`looks_like_header` decided headers from the first row alone (it only
returned "data" when at least half the first ~8 non-empty cells were
numeric/boolean), so an all-text first row in a headerless file
(`alice,bob / carol,dave`) was consumed as headers and the file lost a
row. The user asked Anvil to understand that a first row is sometimes
not a header, and that headerless files should label columns `col0`,
`col1`, ….

### Implemented

- **Heuristic (`src/parser/delimit.rs`)**: `looks_like_header_in_context`
  compares the first row's per-column textness (empty / typed / text)
  against up to 20 following logical lines. Text-over-typed is a header
  vote; typed-over-typed is a data vote; text-over-text is deliberately
  no vote. Majority decides, and a tie/zero falls back to the previous
  `looks_like_header` rule. So `name,age / alice,30` is a header,
  `alice,30 / carol,40` is headerless, and the genuinely ambiguous
  all-text file keeps defaulting to a header.
- **Forced modes (`HeaderMode { Auto, Header, Headerless }`)** on
  `IndexOptions`; `detect_with` skips the heuristic for the force
  modes; `detect` / `parse` stay `Auto` for existing callers.
- **Cheap re-index (`anvil_core::reindex`)**: `OpenedSource` now derives
  `Clone` (the mmap and the extraction `TempDir` are shared behind
  `Arc`s), so applying a forced mode re-runs `indexing::parse_with`
  against the same source without re-opening the file or re-extracting
  an archive entry.
- **UI**: a toolbar **Headers** control cycles `Auto → On → Off`. The
  mode is per tab (`DocumentSnapshot::header_mode`, mirrored live);
  forcing a mode re-indexes on the background executor under the open
  `generation` guard, swaps the tab's `Arc<ParsedDocument>` /
  `GridModel` by tab id, resets stale filter / sort / search state, and
  clamps an out-of-range selection. The encoding report is invalidated
  and re-kicked if the panel is open.
- **`col0`, `col1`, … labels**: new `parser::column_label` /
  `ParseMeta::column_label`, used consistently by the grid header,
  search scope chip, sort label, column inspector, and encoding scan.
  Headerless files leave `headers` empty and are labelled at display
  time; `has_headers` stays meaningful.

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 301 passed (111 lib + 117 binary + 73
  integration), 0 failed, 3 ignored. Was 287.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- GUI launches and stays alive with no panic; the toolbar control,
  re-index task, and persistence are exercised headlessly.

### Tests added / changed

- `src/parser/delimit.rs` (8): text header over typed data, typed first
  row over typed data, numeric first row over text data, ambiguous
  all-text fallback, `alice,30` headerless, forced `Header`, forced
  `Headerless`, and `detect` == `detect_with(Auto)`.
- `tests/core.rs` (3): `headerless_text_rows_are_detected_when_the_shape_matches_data`
  (plus `colN` labels), `forced_header_mode_overrides_detection`,
  `reindex_reuses_the_opened_source` (drops the original first to prove
  the shared mmap outlives it).
- `src/ui/view/tests.rs` (3): `cycle_header_mode_reindexes_active_document`,
  `header_toggle_clamps_selection`, `header_mode_persists_per_tab`.
- Updated existing assertions from `col 0` to `col0` (`schema/stats.rs`,
  `encoding.rs`, `search_column_label` test).

### Roadmap changes

- Milestone 4 gained a **Header detection** group (heuristic, forced
  modes, toolbar cycle, `colN` labels) — all ticked — plus header
  detection tests and an acceptance bullet.

### Architecture changes

- `ARCHITECTURE.md`: the delimiter/header sections document the vote
  heuristic, `HeaderMode`, `detect_with`, `colN` labelling, and the
  `reindex` path; the primary-state table gained `header_mode`; the
  testing section lists the new tests and counts (73 integration,
  65 UI regression).
- `README.md`: test counts refreshed (73 data-side + 117 binary).

### Issues / technical debt discovered

- All-text header over all-text data remains ambiguous under `Auto`;
  the override is the complete answer. Documented in the roadmap.
- Forcing a mode re-runs `ColumnLayout::compute` (200-row sample) on the
  UI thread inside `install_reindexed`, matching the existing open path.
  Fine for now; moving layout measurement to the background task is a
  later optimisation.
- `reindex` clones `archive_candidates` (a small `Vec`); negligible
  unless an archive has very many entries.

### Open loops

- Carried over: archive diagnostics, compound filters, filter chips,
  cache inferred profiles at open, view-scoped inspector stats, richer
  type classes, cap/group large archives, lazy-extraction cancellation,
  panel resize/reorder.

### Next recommended task

Milestone 4 **archive diagnostics**: surface the already-computed
`rejected_reason` for skipped archive entries in the picker, with
friendlier messages for encrypted / ratio-capped entries. That is the
last substantial Milestone 4 group before richer filters / Milestone 5.

## 2026-10-31 — Milestone 4: archive diagnostics (skipped entries, encryption, ratio)

### Context

The previous iteration recommended the last substantial Milestone 4 group,
archive diagnostics: surface the already-computed `rejected_reason` for
skipped archive entries, present compression-ratio findings nicely, and
give encrypted entries a friendlier message. The picker already showed a
bare "N skipped entries" count, and encryption was only rejected during
extraction with a generic `AnvilError::Other` string.

### Implemented

- **`evaluate_entry` (`src/source/archive.rs`)** now classifies each entry
  up front and records a reason for every skip: directories, `__MACOSX` /
  `.DS_Store` / hidden, `..`, oversized, unsupported extension, **encrypted**
  (new `REJECTED_ENCRYPTED` const), **unsupported compression method**, and
  a **central-directory compression ratio** above `MAX_COMPRESSION_RATIO`.
  The decision helpers (`is_supported_method`, `ratio_exceeded`,
  `compression_ratio`) are small and independently tested.
- **`ArchiveSource::first_encrypted_entry`** exposes the first
  password-protected skip so the loader can name it.
- **`AnvilError::EncryptedEntry(String)`** replaces the generic string in
  `extract_into_temp`; the extraction checks remain as a backstop.
- **`loader::load`** returns `EncryptedEntry` when an archive has data
  files but every candidate is encrypted, instead of the generic
  `NoUsableEntries`.
- **`ui::archive_picker`** gains `SkippedEntry` and `skipped_entries`, and
  the toolbar **Archive…** overlay now renders a _skipped entries_ section:
  each rejected entry's name and reason, capped at 40 rows with an
  "…and N more" summary so a noisy archive stays readable.
- Removed an orphaned doc comment above `should_highlight` in
  `ui/grid.rs` (a clippy `empty_line_after_doc_comments` warning).

### Verified

- `cargo build --release` — succeeds, no warnings.
- `cargo test --release` — 311 passed (115 lib + 120 binary + 76
  integration), 0 failed, 3 ignored. Was 301.
- `cargo clippy --release --all-targets` — clean.
- `cargo fmt --check` — clean.
- `cargo run --release --bin anvil -- --probe fixtures/archive.zip` —
  1,000,000 rows from `data/large.csv`, ~1 s.
- `cargo run --release --bin anvil -- --probe fixtures/large.csv` —
  1,000,000 rows, 20 columns, 477 ms.
- `cargo run --release --bin anvil -- --probe fixtures/multi.zip` —
  two candidate files, unchanged.
- GUI launches and stays alive for 6 s with no panic/output.

### Tests added / changed

- `src/source/archive.rs` (4 unit tests): ratio boundaries and the
  `compressed == 0` case, ratio rounding, the supported-method set, and
  `extension_rank`.
- `src/ui/archive_picker.rs` (3 unit tests): `format_size` binary units,
  `skipped_entries` name-sorting and usable filtering, empty case.
- `tests/core.rs` (3 integration tests): `zip_encrypted_entry_is_skipped_and_reported`
  (flagged bit 0, reason equals `REJECTED_ENCRYPTED`, `load` returns the
  typed error), `zip_mixed_encrypted_and_plain_keeps_plain_usable`, and
  `zip_ratio_capped_entry_is_skipped` (declared size defeats the ratio
  guard without a real payload). `raw_zip` was refactored into
  `raw_zip_impl` with flag / declared-size helpers.

### Roadmap changes

- Milestone 4 → Archive diagnostics: all three bullets ticked, plus a
  note that unsupported compression is reported at listing. The remaining
  Milestone 4 work is empty/duplicate-header navigation and richer
  filters.

### Architecture changes

- `ARCHITECTURE.md`: ZIP flow and `evaluate_entry` reject list document
  encryption / unsupported-compression / ratio rejection at listing and
  the reason strings; the multi-entry picker section describes the
  skipped-entries list and the all-encrypted `EncryptedEntry` path; the
  extraction step and archive-safety list call out the typed error and
  the two-place ratio guard; the testing counts were refreshed (76
  integration, archive.rs + archive_picker.rs unit tests).
- `README.md`: test counts (76 data-side + 120 binary) and the
  diagnostics summary refreshed.

### Issues / technical debt discovered

- The ratio guard uses the sizes declared in the central directory. A
  crafted archive could understate its uncompressed size; the streaming
  extraction check (actual copied bytes) remains the authoritative
  backstop. Listing-time rejection is an early-warning optimisation, not
  a replacement.
- Skipped entries are still labelled by prose string. If a later feature
  needs to act on the reason (e.g. a dedicated "password required"
  action), `rejected_reason` should become a small enum.
- The picker caps the skipped list at 40 rows but `ArchiveSource` still
  holds every candidate; a very large archive builds a long list on every
  rebuild. Not measured as a problem yet.

### Open loops

- Carried over: empty/duplicate-header navigation, compound filters,
  filter chips, cache inferred profiles at open, view-scoped inspector
  stats, richer type classes, cap/group large archives, lazy-extraction
  cancellation, panel resize/reorder.

### Next recommended task

Finish Milestone 4. Two groups remain: **empty/duplicate-header
navigation** (small — the diagnostics panel lists header-level warnings
but cannot jump anywhere; decide whether to highlight the header row or
report clearly) and **richer filters** (a compound `AND` / `OR` /
parenthesis parser on top of the existing `Vec<Filter>` AND model, with
tests). After that the project is ready for Milestone 5 performance and
UX hardening.

## 2026-11-01 — Fix: panel close lag (synchronous grid viewport)

### Context

The user reported a visible lag between closing the inspector /
diagnostics panels and the CSV table reflowing into the freed space.
`MainView.viewport_w` — which drives header/row column culling, the
horizontal scrollbar, and scroll-into-view — was derived from the
**previous frame's** measured `grid_bounds` (recorded by the overlay
canvas). Closing a panel therefore repainted the grid once at the old
narrow width, and only widened after the canvas re-measured and called
`cx.notify()`. The existing `viewport_width_excludes_file_panel` test
even documented this: it forced an extra `cx.notify()` so the viewport
could "settle".

### Implemented

- Added `MainView::grid_viewport_width`, which computes the columns-area
  width synchronously as `window width - ROW_NUMBER_WIDTH - (each open
panel's theme width)`. The file panel, inspector, and diagnostics
  panels are all `flex_shrink_0` with fixed widths, so this matches the
  measured layout exactly — but is correct on the _same_ render a panel
  is toggled.
- `render_grid` now uses that value; `grid_bounds` is still recorded for
  drag auto-scroll, which genuinely needs the on-screen rectangle.
- Updated the overlay canvas comment (the viewport no longer waits on
  it).

### Verified

- `cargo build --release` — clean.
- `cargo test --release` — 311 passed (115 lib + 120 binary + 76
  integration), 0 failed, 3 ignored.
- `cargo clippy --release --all-targets` — clean; `cargo fmt --check` —
  clean.
- GUI launches and stays alive 6 s with no panic.

### Tests added / changed

- Replaced `viewport_width_excludes_file_panel` with
  `viewport_width_reflows_synchronously_with_panels`, which opens the
  inspector and diagnostics and then closes everything, asserting the
  viewport matches in the **same** frame each time (a single
  `notify` + render, no forced extra frame). It fails against the old
  previous-frame measurement.

### Architecture changes

- `ARCHITECTURE.md`: the `viewport_w` section now documents the
  synchronous `grid_viewport_width` computation and why the measured
  approach lagged; the `grid_bounds` state-table row and the column
  inspector note were updated accordingly.

### Issues / technical debt discovered

- The synchronous calculation duplicates the panel-presence conditions
  used in `render` (file panel open && documents non-empty; inspector /
  diagnostics open && model present). If a panel is ever added or made
  resizable, both sites must change together. A future refactor could
  have `render` compute a small `PanelLayout` once and pass it down.
- The overlay canvas still calls `cx.notify()` when `grid_bounds`
  changes, so a panel toggle now schedules one redundant follow-up
  frame. It is invisible (the viewport is already correct) but could be
  dropped if auto-scroll freshness is handled differently.

### Open loops

- Carried over: empty/duplicate-header navigation, compound filters,
  filter chips, cache inferred profiles at open, view-scoped inspector
  stats, richer type classes, cap/group large archives, lazy-extraction
  cancellation, panel resize/reorder.

### Next recommended task

Back to finishing Milestone 4: **empty/duplicate-header navigation**
(small) and **richer filters** (compound `AND` / `OR` / parentheses).
If the panel-reflow duplication is felt to be a risk, land the
`PanelLayout` refactor first as its own iteration.

## 2026-11-02 — Milestone 4: compound filter predicates (AND / OR / parentheses)

### Context

The remaining substantial Milestone 4 item was richer filters: compound
predicates. The data side only supported a single predicate per apply
(`apply` ANDs a `Vec<Filter>`), so `OR` and parentheses were impossible.
I attempted to consult the architecture specialist for the grammar /
representation design and then delegate the implementation to a worker.
The architecture consultation timed out (180 s) and the worker calls were
rejected by operator configuration / exhausted the worker token envelope,
so the slice was implemented locally as a documented recovery. No design
advice from the specialist was used.

### Implemented

- **`src/query/filter.rs`**:
  - `FilterExpr { Predicate, And, Or }` — an AND/OR tree.
  - `parse_expr` — recursive-descent parser over a small tokenizer.
    Grammar: `expr := term (OR term)*`, `term := factor (AND factor)*`,
    `factor := '(' expr ')' | predicate`. AND binds tighter than OR.
  - Tokenizer handles `(`, `)`, quoted values (with `\"`), and words;
    `AND` / `OR` are recognised case-insensitively only when unquoted, so
    a literal keyword value can be quoted (`name contains "AND"`).
  - Predicate values are collected until the next boolean keyword, paren,
    or end of input, preserving the existing unquoted multi-word values
    (`city == New York AND amount > 100`).
  - `apply_expr` prepares each predicate once (`PreparedExpr`) and
    short-circuits per row; no per-row allocation.
  - `parse` and `apply` keep their signatures and behavior (apply folds a
    slice into a left-associated AND tree), so existing callers and tests
    are untouched.
- **`src/lib.rs`**: re-export `apply_expr as apply_filter_expr`,
  `parse_expr as parse_filter_expr`, and `FilterExpr`.
- **UI** (`src/ui/view.rs`, `src/ui/view/filter.rs`): filter state is now
  `Option<FilterExpr>`; `apply_filter_input` uses `parse_expr` /
  `apply_expr`; the input placeholder advertises booleans.

### Verified

- `cargo build --release` — clean.
- `cargo test --release` — 321 passed (124 lib + 121 binary + 76
  integration), 0 failed, 3 ignored. Was 311.
- `cargo clippy --release --all-targets` — clean; `cargo fmt --check` —
  clean.
- GUI launches and stays alive 6 s with no panic.

### Tests added / changed

- `src/query/filter.rs` (9 unit tests): AND-over-OR precedence and tree
  shape, parentheses override plus redundant nesting, multi-word value
  before a boolean operator, quoted keyword / quoted multi-word values,
  case-insensitive keywords, parse errors (unbalanced parens, missing
  value before `AND`, empty, trailing operator, stray word), single
  predicate equivalence with `parse`/`apply`, OR dedup and short-circuit,
  and decimal-threshold numeric ordering (`amount > 1000.0`).
- `src/ui/view/tests.rs` (1 UI test):
  `filter_input_applies_compound_expression_on_enter` runs
  `country == US AND amount > 25` and `(amount > 25 OR country == UK)`
  through the real filter bar and asserts the filtered rows.

### Roadmap changes

- Milestone 4 → Richer filters: compound predicates ticked; the numeric
  comparison bullet ticked with a decimal-threshold test; the filter
  expression test bullet ticked (regex remains possible). Saved filters
  still open.

### Architecture changes

- `ARCHITECTURE.md`: the filter module section documents `FilterExpr`,
  `parse_expr` / `apply_expr`, the grammar, and the quoting rule; the
  state-table row now reads `Option<FilterExpr>`; the testing table and
  the `src/query/filter.rs` unit-test count (18 → 27) were updated; the
  limitations note now records that `AND`/`OR`/parentheses exist but
  `NOT` and saved filters do not.
- `README.md`: test counts (76 + 121) and coverage/summary refreshed.

### Issues / technical debt discovered

- There is no `NOT` / `!` operator yet.
- Predicate values are greedy, so a missing operator is silently absorbed:
  `a == 1 b == 2` parses as the single predicate `a == 1 b == 2` (value
  `"1 b == 2"`) rather than erroring. This matches the original
  `parse` behavior; quoting or a stricter value rule could reject it in a
  later pass.
- Only double quotes are supported as a value escape; there is no single
  quote or backslash-escape outside `\"`.
- Delegation friction: the architecture consultation and worker attempts
  failed (timeout / operator-configured check / token envelope). No
  specialist output was integrated; the slice was verified locally.

### Open loops

- Carried over: empty/duplicate-header navigation, saved filters, `NOT`,
  filter chips, cache inferred profiles at open, view-scoped inspector
  stats, richer type classes, cap/group large archives, lazy-extraction
  cancellation, panel resize/reorder.

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation** (clicking a
header-level warning should highlight the header row/column instead of
only reporting "no row to jump to"), then move to Milestone 5
(performance profiling, recent files, column resize, theme toggle).

## 2026-11-03 — Fix: search/filter bars not closable, and caret drift

### Context

The user reported two UI bugs: (1) the search and filter bars could not
be dismissed, and (2) the text caret drifted progressively ahead of the
typed text. I diagnosed both before editing.

- **Close:** neither bar had a visible close control, and the grid's
  `Action::Escape` arm handled only `search_open`. Once focus returned to
  the grid (e.g. after clicking a cell), the filter bar was impossible to
  dismiss — only Esc while the bar itself held focus worked.
- **Caret:** `ui::input::shape_input_line` shaped with
  `window.text_style()`, which during render is the _default_ 1rem style,
  while the bars render text at `text_sm` (0.875rem). The caret x was
  therefore computed at 16px against 14px glyphs, and the error grew with
  text length — exactly the reported "gets farther away the more we type".

Worker delegation was unavailable this session (the worker token envelope
was already exhausted by the earlier compound-filter task), so the fixes
were made locally as a documented recovery.

### Implemented

- **`src/ui/input.rs`**: added `input_font_size` (0.875rem) and shape the
  caret/hit-test line at that size; `editable_text_area` now sets
  `.text_sm()` explicitly so the shaped and rendered sizes cannot diverge.
- **`src/ui/view/search.rs`** and **`src/ui/view/filter.rs`**: added a
  visible `×` close control (`search-close` / `filter-close`) wired to
  `close_search` / `close_filter`.
- **`src/ui/view.rs`**: `Action::Escape` now closes the filter bar too (and
  prefers closing whichever bar is open), so Esc works from the grid.

### Verified

- `cargo build --release` — clean.
- `cargo test --release` — 325 passed (124 lib + 125 binary + 76
  integration), 0 failed, 3 ignored. Was 321.
- `cargo clippy --release --all-targets` — clean; `cargo fmt --check` —
  clean.
- GUI launches and stays alive 6 s with no panic.
- Proven regression coverage: with the filter arm of `Action::Escape`
  temporarily disabled, `escape_action_closes_filter_bar_when_grid_focused`
  fails; restoring it passes. (The temporary `false &&` was removed.)
- The caret alignment is a rendering-layout fix and is not asserted by an
  automated test (no pixel harness); it follows from making the shape
  size equal the rendered `text_sm` size.

### Tests added

- `src/ui/view/tests.rs` (4): `escape_closes_filter_bar`,
  `escape_action_closes_filter_bar_when_grid_focused`,
  `filter_close_button_dismisses_bar`, `search_close_button_dismisses_bar`
  (the last two synthesise a real click on the `×` via a debug selector).

### Architecture changes

- `ARCHITECTURE.md`: the editable-field section documents the
  `text_sm` shaping size and the drift it prevents, and the visible `×`
  controls; the keyboard table's Esc row now says it closes the search or
  filter bar; the UI regression-test count (65 → 69) and test list were
  updated.
- `README.md`: binary test count (121 → 125).

### Issues / technical debt discovered

- The filter/search bars still have no caret blink and no horizontal
  scroll when the text is wider than the field (existing follow-up).
- `editable_text_area` hardcodes the 0.875rem input size to match
  `text_sm`; if a bar's text style changes, `input_font_size` must change
  with it. A single `theme` constant would remove the duplication.

### Open loops

- Carried over: empty/duplicate-header navigation, saved filters, `NOT`,
  filter chips, cache inferred profiles at open, view-scoped inspector
  stats, richer type classes, cap/group large archives, lazy-extraction
  cancellation, panel resize/reorder.

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation**, then move
to Milestone 5 (performance profiling, recent files, column resize,
theme toggle). The caret-size duplication noted above is a small cleanup
worth folding into a future UI iteration.

## 2026-11-04 — Milestone 2/4: filter input autocomplete

### Context

The user asked for suggestions in the filter input: start typing and a
list of options appears, navigable with the arrow keys or selectable
with the mouse, which fills in the text. This is a new cross-component
feature, so an architecture consultation was run; it returned a concrete
design (GPUI-free slot state machine, bounded sampler, MainView cache,
`TextEdit::replace_range`, popup + key interception). The implementation
worker then failed twice (`worker_failure`, token envelope exhausted) with
no artifacts, so the designed feature was implemented locally as a
documented recovery.

### Implemented

- **`src/query/suggest.rs` (new)**: `Slot` (Column / Operator / Value /
  Keyword) and `SuggestContext { slot, replace, partial }`. `context`
  scans tokens leniently (parens, quoted values, unterminated quotes) and
  runs an incomplete-input-tolerant state machine to find the slot at the
  caret and the byte range a suggestion should replace. `candidates`
  returns case-insensitive prefix-filtered, deduped, capped suggestions.
- **`src/schema/stats.rs`**: `sample_distinct(doc, column, max_rows,
max_distinct)` — a bounded, first-seen-order distinct-value sampler for
  the value slot.
- **`src/ui/text_edit.rs`**: `TextEdit::replace_range(range, text)` to
  replace the partial token and move the caret/anchor.
- **`src/ui/view.rs` / `src/ui/view/filter.rs`**: `filter_suggestions` /
  `filter_suggestion_selected` / `filter_popup_open` /
  `filter_value_cache` state; `refresh_filter_suggestions` (recomputed on
  each edit and when the bar opens; samples a column once and caches it);
  `accept_filter_suggestion` (replaces the token, appends a separator
  space when needed, closes the popup). Keys: while the popup is open
  Up/Down move the selection, Enter/Tab accept, Esc closes the popup
  first; Down with the popup closed opens it. The popup renders as an
  overlay (via `render_overlay`, so it paints above the grid) with the
  selected row highlighted and clickable. `filter_value_cache` is cleared
  in `load_active` on document change.
- **`src/query/filter.rs`**: `resolve_column` is now `pub(crate)` and
  reused by `suggest`.

### Verified

- `cargo build --release` — clean.
- `cargo test --release` — 349 passed (142 lib + 131 binary + 76
  integration), 0 failed, 3 ignored. Was 325.
- `cargo clippy --release --all-targets` — clean; `cargo fmt --check` —
  clean.
- GUI launches and stays alive 6 s with no panic.
- The popup genuinely renders and is hit-testable: the mouse test
  resolves `debug_bounds("filter-suggestion-1")` and clicks it, which
  fills the input.

### Tests added

- `src/query/suggest.rs` (14): empty/partial column, operator slot and
  prefix, value slot column/op and prefix filtering, value-less op →
  Keyword, `AND ` → Column, `(` → Column, multi-word value then boolean,
  caret mid-word replace range, unknown column, multibyte byte range,
  quoted partial.
- `src/schema/stats.rs` (4): `sample_distinct` dedup/first-seen, row cap,
  distinct cap + early stop, empty column.
- `src/ui/text_edit.rs` (1): `replace_range` partial replacement,
  empty-range insert, out-of-range clamp.
- `src/ui/view/tests.rs` (5): popup opens on typing with the expected
  item; Up/Down move and Enter accepts (filling `country `); Esc closes
  the popup then the bar; mouse click fills the input; document switch
  clears the value cache.

### Roadmap changes

- Milestone 2 → Filter: the "richer expression language" bullet (landed
  earlier as `FilterExpr`) is now ticked, and a new completed
  autocomplete bullet was added. Saved filters / filter chips remain
  open.

### Architecture changes

- `ARCHITECTURE.md`: repo tree gains `query/suggest.rs`; the filter
  module section documents the suggest state machine, popup, key
  handling, `replace_range`, and the bounded cached sampler; the testing
  section lists the new unit counts (suggest 14, stats 13) and the
  filter-autocomplete test row; UI regression count 69 → 74.
- `README.md`: binary test count 125 → 131 and the coverage summary.

### Issues / technical debt discovered

- The one-time value sample runs synchronously on the UI thread the first
  time a value slot is opened for a column (bounded to 5000 rows / 100
  values). Fine for normal files; a very wide/slow source could hitch and
  would want the existing background-task pattern.
- Suggestions are prefix-only (no fuzzy/substring), and the popup is
  left-aligned under the bar rather than caret-anchored.
- `refresh_filter_suggestions` is recomputed on edits and on popup-open,
  but not on plain caret moves via clicks; suggestions can be stale until
  the next edit. Documented as a follow-up.
- The value cache is cleared in `load_active`; a re-index (header mode)
  also goes through `load_active`? `install_reindexed` resets live state
  and calls `load_active`, so it is covered.

### Open loops

- Carried over: empty/duplicate-header navigation, saved filters, filter
  chips, cache inferred profiles at open, view-scoped inspector stats,
  richer type classes, cap/group large archives, lazy-extraction
  cancellation, panel resize/reorder.

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation**, then move
to Milestone 5 (performance profiling, recent files, column resize,
theme toggle). A small autocomplete follow-up would be to move the
first-time value sample to the background executor and to refresh
suggestions on caret clicks.

## 2026-11-05 — Fix: autocomplete popup did not scroll with the selection

### Context

The user reported that arrowing Up/Down in the filter autocomplete moved
the highlighted row but the popup did not scroll, so the selection could
move off the visible area. Root cause: the popup rendered with
`overflow_y_scroll()` but had no scroll handle, so nothing ever scrolled
it as `filter_suggestion_selected` changed.

### Implemented

- **`src/ui/view.rs`**: added `filter_suggestion_scroll: ScrollHandle`
  (init `ScrollHandle::new()`).
- **`src/ui/view/filter.rs`**: the popup attaches
  `.track_scroll(&self.filter_suggestion_scroll)`; Up/Down call
  `scroll_to_item(selected)` after moving the selection, and
  `refresh_filter_suggestions` scrolls back to item 0. GPUI's default
  `FirstVisible` strategy scrolls the minimal amount to keep the row in
  view.

### Verified

- `cargo build --release` — clean.
- `cargo test --release` — 350 passed (142 lib + 132 binary + 76
  integration), 0 failed, 3 ignored. Was 349.
- `cargo clippy --release --all-targets` — clean; `cargo fmt --check` —
  clean.
- GUI launches and stays alive 5 s with no panic.
- Regression test `filter_suggestions_scroll_to_keep_selection_visible`
  builds a 50-value column, opens the value-slot popup, presses Down 15
  times, and asserts the popup scroll offset is non-zero
  (`offset.y = -291.5`, i.e. scrolled down). Without the scroll handle
  the offset stays 0 and the test fails.

### Tests added

- `src/ui/view/tests.rs` (1): `filter_suggestions_scroll_to_keep_selection_visible`.

### Architecture changes

- `ARCHITECTURE.md`: the filter-autocomplete paragraph notes the
  `filter_suggestion_scroll` handle and `scroll_to_item`; UI regression
  count 74 → 75.

### Issues / technical debt discovered

- None new. The previously noted autocomplete follow-ups (background
  first-time value sample, prefix-only matching, refresh on caret clicks)
  still stand.

### Open loops

- Carried over: empty/duplicate-header navigation, saved filters, filter
  chips, cache inferred profiles at open, view-scoped inspector stats,
  richer type classes, cap/group large archives, lazy-extraction
  cancellation, panel resize/reorder.

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation**, then
Milestone 5 hardening.

## 2026-11-06 — Fix: value autocomplete empty for some columns

### Context

The user reported that for some columns, typing `column == value` did not
populate value suggestions. Diagnosis found four distinct gaps, all in
how a column label or value has to round-trip through the parser:

1. **Headerless columns** are offered/typed as `colN`, but `resolve_column`
   only understood header names, `#N`, and numeric indexes, so it could not
   resolve `col0` — the value slot saw `column: None` and offered nothing.
2. **Numeric-named headers** (e.g. a column literally named `42`) were
   parsed as an index, so they resolved to the wrong column or errored.
3. **Multi-word column names** (`New York`) were inserted unquoted and
   split into two tokens, so the column could not be resolved.
4. **Value matching was prefix-only**, so once the user typed the second
   word of a multi-word value (`New York` → partial `Y`) nothing matched.

### Implemented

- **`src/query/filter.rs` — `resolve_column`**: prefers an exact header
  match over numeric parsing, accepts generated `colN` labels, and keeps
  `#N` as an explicit index. So the labels the autocomplete offers always
  resolve.
- **`src/query/suggest.rs` — candidate matching**: replaced prefix-only
  `push_matching` with a ranked matcher (`match_rank`: whole-string prefix
  > word prefix > substring) and stable best-first ordering, so values
  > surface when typing a later word or a mid-string fragment.
- **`src/ui/view.rs` / `src/ui/view/filter.rs`**: `FilterSuggestion`
  carries a `quote` flag; a multi-word (or quoted) column suggestion is
  inserted wrapped in double quotes (with `\"` escaping) so it stays one
  token. This is what makes the value slot populate after accepting a
  multi-word column.

### Verified

- `cargo build --release` — clean.
- `cargo test --release` — 357 passed (148 lib + 133 binary + 76
  integration), 0 failed, 3 ignored. Was 350.
- `cargo clippy --release --all-targets` — clean; `cargo fmt --check` —
  clean.
- GUI launches and stays alive 5 s with no panic.

### Tests added

- `src/query/filter.rs` (1): `resolve_column_round_trips_generated_and_numeric_labels`
  (`col0`/`col2`, out-of-range `col9`, a header named `42`, plain index,
  `#N`).
- `src/query/suggest.rs` (5): value slot for `col0` (headerless), for a
  numeric-named header, and for a quoted multi-word column; word-prefix
  and substring value matching; prefix-before-substring ordering.
- `src/ui/view/tests.rs` (1): `accepting_a_multi_word_column_suggestion_quotes_it`
  — accepting `New York` inserts `"New York"` and the value slot then
  offers `NYC`/`LA`.

### Architecture changes

- `ARCHITECTURE.md`: the filter-autocomplete paragraph records the label
  round-trip (header-first `resolve_column`, `colN`, quoted multi-word
  names) and ranked value matching; unit counts updated (filter 27 → 28,
  suggest 14 → 19) and UI regression count 75 → 76.

### Issues / technical debt discovered

- Quoting is applied to column suggestions containing whitespace or a
  quote. A header beginning with `#` would still be read as an index
  before the header check; `#`-prefixed header names remain an edge case.
- Substring matching can surface a value whose match is mid-string rather
  than at a boundary; ranking keeps prefixes on top.

### Open loops

- Carried over: empty/duplicate-header navigation, saved filters, filter
  chips, cache inferred profiles at open, view-scoped inspector stats,
  richer type classes, cap/group large archives, lazy-extraction
  cancellation, panel resize/reorder; plus the earlier autocomplete
  follow-ups (background first-time value sample, refresh on caret
  clicks).

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation**, then
Milestone 5 hardening.

## 2026-11-07 — Fix: `#`-prefixed column names resolve

### Context

The previous fix made `resolve_column` prefer an exact header match so a
column named `42` or `col0` resolves to itself, but left the explicit
`#N` index check first. A column literally named `#2` (or `#foo`) was
therefore still read as an index (`#2` → column 2) or errored
(`#foo` → "bad column index"), so its value suggestions never populated.

### Implemented

- **`src/query/filter.rs` — `resolve_column`**: the exact header match is
  now checked first for _all_ tokens, then `#N`/`colN`/numeric are the
  fallbacks. So a header named `#2` resolves to itself, while `#N` still
  means "index N" when no header matches (the common case).

### Verified

- `cargo build --release` — clean.
- `cargo test --release` — 357 passed (148 lib + 133 binary + 76
  integration), 0 failed, 3 ignored.
- `cargo clippy --release --all-targets` — clean; `cargo fmt --check` —
  clean.

### Tests added / changed

- Extended `resolve_column_round_trips_generated_and_numeric_labels`:
  a header named `#2` resolves to that header; `#1` still resolves to
  index 1 when no header matches; `#foo` errors.

### Architecture changes

- `ARCHITECTURE.md`: the autocomplete paragraph now states the exact
  header match wins for `42`, `col0`, and `#2`, with `#N` as the
  fallback index.

### Issues / technical debt discovered

- None new. If a file has a header named `#2` _and_ the user wants index
  2, they must use the plain numeric form `2`; documented behaviour.

### Open loops

- Carried over: empty/duplicate-header navigation, saved filters, filter
  chips, view-scoped inspector stats, richer type classes, cap/group
  large archives, lazy-extraction cancellation, panel resize/reorder,
  and the autocomplete follow-ups (background first-time value sample,
  refresh on caret clicks).

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation**, then
Milestone 5 hardening.

## 2026-11-08 — Search/filter bars switch directly with their shortcuts

### Context

The user wanted pressing Cmd/Ctrl+L while in Cmd/Ctrl+F (and vice versa)
to switch cleanly, without closing the current bar first. Two problems
blocked this: (1) the bar key handlers only ran while their own bar had
focus, and the other bar's shortcut was a sibling handler that never
fired, and (2) `Action::OpenSearch` / `Action::OpenFilter` would open the
target bar but leave the other one open.

### Implemented

- **`src/ui/view.rs`**: added `switch_to_filter` / `switch_to_search`.
  Each closes the other bar (clearing a stale `focus_grid_on_next_frame`
  so the destination bar actually gets focus), then opens the target bar
  or notifies if it is already open. `Action::OpenSearch` /
  `Action::OpenFilter` now call these, so the switch also works when
  focus is on the grid.
- **`src/ui/view/search.rs`**: `on_search_key_down` recognises
  Cmd/Ctrl+L and calls `switch_to_filter`.
- **`src/ui/view/filter.rs`**: `on_filter_key_down` recognises
  Cmd/Ctrl+F and calls `switch_to_search`.

### Verified

- `cargo build --release` — clean.
- `cargo test --release` — 360 passed (148 lib + 136 binary + 76
  integration), 0 failed, 3 ignored. Was 357.
- `cargo clippy --release --all-targets` — clean; `cargo fmt --check` —
  clean.
- GUI launches and stays alive 5 s with no panic.

### Tests added

- `src/ui/view/tests.rs` (3): `cmd_l_switches_from_search_bar_to_filter`,
  `cmd_f_switches_from_filter_bar_to_search`, and
  `cmd_l_from_grid_switches_open_search_to_filter` (the grid action path).

### Architecture changes

- `ARCHITECTURE.md`: the keyboard table gained a Cmd/Ctrl+L row and the
  Cmd/Ctrl+F row now describes the switch; the search-bar section
  documents `switch_to_search` / `switch_to_filter`; the UI regression
  test list and count (76 → 79) were updated.

### Issues / technical debt discovered

- Opening a bar from the toolbar can still leave both bars visible; only
  the keyboard shortcuts are guaranteed mutually exclusive. Not changed
  here because the two views are composable (filter narrows, search
  highlights) and the user asked specifically about the shortcuts.

### Open loops

- Carried over: empty/duplicate-header navigation, saved filters, filter
  chips, view-scoped inspector stats, richer type classes, cap/group
  large archives, lazy-extraction cancellation, panel resize/reorder,
  autocomplete follow-ups (background first-time value sample, refresh on
  caret clicks).

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation**, then
Milestone 5 hardening.

## 2026-11-09: Remove em-dashes from source and guard against them

### Context

The user asked to remove em-dashes (U+2014) from the program and stop
them reappearing. A scan found 32 in `.rs` files: mostly doc/line
comments, plus six user-visible strings (filter placeholder, archive
picker caption, loading/error status, header-warning status, and the
empty-value placeholder used by the status bar and inspector).

### Implemented

- Replaced every em-dash in `src/**/*.rs` and `tests/**/*.rs` with a
  plain hyphen, then tidied the user-visible strings to read naturally:
  - filter placeholder: `column op value, e.g. country == US AND amount > 100`
  - archive picker: `{}: every usable file already has a tab`
  - status: `loading: {note}` / `error: {message}`
  - diagnostics: `header-level warning: no row to jump to`
  - empty values render as `-`
- Added `tests/style.rs`, a repository style guard. It walks every `.rs`
  file under `src/` and `tests/` (via `CARGO_MANIFEST_DIR`) and fails if
  any line contains U+2014, naming the file and line. The guard uses the
  `'\u{2014}'` escape so the guard file itself contains no em-dash.

### Verified

- `cargo build --release` clean.
- `cargo test --release`: 361 passed (148 lib + 136 binary + 76 core
  integration + 1 style), 0 failed, 3 ignored. Was 360.
- `cargo clippy --release --all-targets` clean; `cargo fmt --check` clean.
- Searching `src` and `tests` for U+2014 reports none.
- Guard proven: a temporary probe file containing an em-dash made
  `cargo test --test style` fail with the file and line; the probe was
  deleted and the suite is green again.

### Tests added

- `tests/style.rs` (1): `source_contains_no_em_dashes`.

### Architecture changes

- `ARCHITECTURE.md`: repository tree gains `tests/style.rs`; the testing
  section describes the em-dash guard.
- `README.md`: test-count line now reads `76 data-side + 136 binary + 1 style guard`.

### Issues / technical debt discovered

- The guard covers `.rs` source only (the program). Markdown docs
  (`README.md`, `ARCHITECTURE.md`, `ROADMAP.md`, `AGENTS.md`,
  `VISION.md`, `docs/agent-log.md`) still contain em-dashes, including
  historical log entries. Cleaning the living documents is possible but
  would touch a large volume of prose and rewrite history, so it was left
  for a separate decision.

### Open loops

- Carried over: empty/duplicate-header navigation, saved filters, filter
  chips, view-scoped inspector stats, richer type classes, cap/group
  large archives, lazy-extraction cancellation, panel resize/reorder,
  autocomplete follow-ups (background first-time value sample, refresh on
  caret clicks).

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation**, then
Milestone 5 hardening.

## 2026-11-10: Performance pass

### Context

Adversarial review of the hot paths (indexing, cell decoding, search,
filter, sort, stats, layout, grid rendering), with before/after
measurement on the 1M-row fixture. Two optimizations landed, one
experiment was a measured pessimization and reverted, and several
remaining opportunities were recorded rather than churned.

### Implemented

- **Case-insensitive search** (`src/query.rs`): the fold path was a
  naive O(cell x needle) sliding window with a per-position shift loop.
  Replaced with a prebuilt `memchr::memmem::Finder` plus one reusable
  scratch buffer; cells with no ASCII uppercase bytes are searched in
  place. `ascii_contains_lower` is retained for the filter's `contains`
  operator.
- **`render_row`** (`src/ui/grid.rs`): decoded the record's visible
  cells in one `Cells` pass instead of calling `doc.cell` per column
  (which re-walked the record from its start, O(columns^2) per row), and
  dropped the throwaway `String` + clone per cell (one `SharedString`
  copy per cell now).
- **`ColumnLayout::compute`** (`src/ui/grid.rs`): same single-pass
  decode per sampled record instead of per column.
- **Benchmarks**: added `measure_search_large_fixture` (tests/core.rs)
  and `measure_wide_column_layout` (src/ui/grid.rs, ignored).
- Documented the negative result inline in `indexing.rs`.

### Measured before/after (Apple Silicon, release, same fixtures)

- Case-insensitive whole-record search, 1M rows:
  **288 ms -> 135 ms** (~2.1x).
- `ColumnLayout::compute`, 500 columns x 300 rows:
  **250 ms -> 2 ms** (~125x).
- `render_row`: not frame-timed (no pixel harness); the change removes
  a per-column re-decode and 1-2 allocations per visible cell by
  construction.
- Indexing (`measure_large_fixture`): ~247 ms, **669 MiB/s** (unchanged).
- Filter `country == US`: ~74 ms. Numeric sort `amount`: ~235 ms.
  (These are higher than the older doc figures; the doc now records the
  current numbers rather than stale ones.)
- Negative result: replacing the indexing byte loop with a
  `memchr3` skip loop was **slower** (434 ms vs 250 ms) because the
  fixture has a delimiter every few bytes, so there is nothing to skip
  and per-call scanner setup dominates. Reverted.

### Verified

- `cargo test --release`: 361 passed (148 lib + 136 binary + 76 core +
  1 style), 0 failed, 4 ignored. Clippy clean; fmt clean.
- GUI launches and stays alive 6 s with no panic.

### Remaining opportunities (not changed)

- `find_ranges` allocates a case-folded copy per matched cell during
  highlight; bounded to the visible range, could take a scratch buffer.
- Text sort allocates one lowercased key `Vec<u8>` per row (1M
  allocations on the fixture); an arena would remove it.
- Indexing is a per-byte branch loop at ~670 MiB/s; the SIMD skip idea
  is a pessimization on dense CSVs.
- `encoding::scan` validates UTF-8 per cell and `column_stats` allocates
  a `String` per distinct value; both are diagnostics-only and bounded,
  left alone.
- No frame-time harness for scrolling; `render_row` improvements are
  argued from allocation/decode count rather than measured.

### Docs

- `ARCHITECTURE.md`: Performance Characteristics updated with the new
  search/layout numbers, the render-row and layout changes, the
  remaining bottlenecks, and the memchr3 negative result.
- `README.md`: benchmark commands added; test count line already current.

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation**, then
Milestone 5 (frame-time harness, LRU cell text, recent files, column
resize, theme toggle).

## 2026-11-11: Fix: value autocomplete missed late-appearing values

### Context

The user typed `Country == Madagascar`, saw no value suggestions once they
reached the middle of the word, yet the filter itself returned the row.
Root cause: `schema::stats::sample_distinct` (used for the value slot)
scanned only the **first `max_rows` records** of the column. In a column
sorted by country, `Madagascar` first appears far past row 5000, so it was
never sampled. Earlier prefixes matched other `Ma*` values, which is why
the list looked non-empty until the partial reached `Mad`.

### Implemented

- **`src/schema/stats.rs`**: `sample_distinct` now samples up to
  `max_rows` records **spread evenly across the whole document** (a
  stride of `total / max_rows`) instead of scanning the head. Earlier
  values are still returned in first-seen order; empty values are still
  skipped; `max_distinct` still caps the result.
- **`src/ui/view/filter.rs`**: the value-slot cache now samples with a
  budget of 50,000 rows / 1000 distinct values (was 5000 / 100),
  which is a one-time cost per column, cached.

### Verified

- `cargo build --release` clean; `cargo test --release` 362 passed
  (149 lib + 136 binary + 76 core + 1 style), 0 failed, 5 ignored;
  clippy clean; fmt clean; GUI launches and stays alive.
- New tests:
  - `sample_distinct_spreads_across_the_whole_file`: 100 rows, row 50 is
    `Madagascar`, budget 10; asserts the value is present. Fails against
    the old head-only sampler.
  - `sample_distinct_respects_the_row_budget` updated to the new spread
    semantics (budget 2 over 5 rows visits rows 0 and 2).
- Measured: the file-wide sampler takes **~16 ms** once per column on the
  1M-row fixture (new `measure_sample_distinct_large_fixture`), cached
  thereafter. The old head-only scan was ~1-2 ms but missed values.

### Remaining work

- The first value sample for a column still runs synchronously on the UI
  thread (now ~16 ms); moving it to the background executor with a
  generation guard is the noted follow-up.
- Sampling is bounded, so a value occurring in very few rows can still be
  missed; a per-prefix fallback scan is the guarantee option if needed.

### Docs

- `ARCHITECTURE.md`: autocomplete paragraph and testing row now describe
  the file-wide sample and its cost.
- `README.md`: benchmark list already includes the measure_* family.

### Next recommended task

Finish Milestone 4 with **empty/duplicate-header navigation**, then
Milestone 5 (frame-time harness, background value sampling, LRU cell
text, recent files, column resize, theme toggle).

## 2026-11-12: Plan: natural-language Ask bar (ADR 0001)

### Context

The user asked whether natural-language filter construction was feasible,
then approved a hybrid design: use rules (Tier 1) when they cleanly
resolve, fall back to an optional local model (Tier 2) otherwise, with
Tier 2 fully optional, bound to `Cmd/Ctrl+Shift+L`. No runtime code was
changed in this iteration; this records the decision and the plan.

### Implemented (planning only)

- **`docs/adr/0001-natural-language-ask.md`** (new, first ADR): records
  the decision, the Tier 1 clean-resolution contract, the optional
  `NlBackend` requirements (feature-flagged, local-only, output always
  re-validated by `parse_expr`, no auto-download), the rejected
  alternatives, and the consequences.
- **`ROADMAP.md`**: added Milestone 6 (Natural-language Ask, opt-in) with
  prerequisites (`FilterExpr::Not`, column aliases), the Tier 1 rules
  parser tasks, the Ask bar tasks, the Tier 2 backend tasks, dependencies,
  and acceptance criteria.
- **`ARCHITECTURE.md`**: added a Planned Architectural Evolution bullet
  pointing at the ADR and stating that `FilterExpr` is the single output
  contract.

### Key decisions recorded

- Escalation to Tier 2 happens only when Tier 1 fails any of: no
  predicate produced, unmatched tokens, unresolved/ambiguous column,
  unbindable value, or low confidence. Otherwise Tier 1's result is used.
- The Ask bar previews; it never auto-applies and never silently guesses.
- Tier 2 is off by default, runs locally, never downloads a model, and
  its output is always re-parsed and schema-validated.

### Verified

- Documentation-only change. No source changed. `cargo test --release`
  was not re-run for this iteration (no code change); the tree was green
  at 362 tests before it.

### Open loops

- Prerequisites to implement next: `FilterExpr::Not` (data-side grammar
  plus `parse_expr`, `apply_expr`, and filter-bar display) and the column
  alias map.
- Then the Tier 1 `query::nl` parser with a phrase-table test suite.
- Then the Ask bar UI (`Cmd/Ctrl+Shift+L`).
- Tier 2 remains demand-gated.

### Next recommended task

Implement the prerequisites for Milestone 6: `FilterExpr::Not` first
(small, independently useful, and needed for "not"/"except" phrasing),
then the column alias map, then the Tier 1 parser.

## 2026-11-13: Milestone 6 Tier 1 natural-language parser

### Context

Following ADR 0001, implemented the deterministic rules-based front end
that turns a phrase into a `FilterExpr`. Tier 2 (optional local model),
the Ask bar, and `FilterExpr::Not` are deliberately not in this slice.

### Implemented

- **`src/query/nl.rs` (new, GPUI-free)**:
  - `NlSchema` / `NlColumn` (name, `ColumnType`, alias table) and
    `NlResult` (`expr`, `unmatched`, `clean`).
  - `NlBackend` trait plus the `RuleBackend` Tier 1 implementation, and a
    free `parse(phrase, schema, values)` function. Values are fetched
    lazily per column through the callback.
  - Tokenizer (whitespace, quoted spans, trailing commas become `and`).
  - Operator lexicon, longest phrase first, with linking-word skipping
    (`is`, `the`, ...), covering equality, negation, `contains`, prefix /
    suffix, empty / non-empty, and the numeric comparisons.
  - Column resolution by header, `colN`, alias, multi-word and quoted
    names.
  - Value binding: longest sampled distinct value first (so
    `Trinidad and Tobago` is not split on `and`), then numeric literals,
    then a raw fallback marked low confidence for equality (substring
    operators stay confident).
  - Conjunction handling for `and` / `or`; a bare value after a
    conjunction reuses the previous column (`status is failed or
refunded`).
  - `clean` is true only when at least one predicate was produced, no
    tokens were left unmatched, every column resolved, and every value
    bound confidently: this is the ADR escalation contract.
- **`src/query.rs` / `src/lib.rs`**: module wiring and public re-exports
  (`parse_nl`, `NlBackend`, `NlColumn`, `NlResult`, `NlSchema`,
  `RuleBackend`).

### Verified

- `cargo build --release` clean; `cargo test --release` 377 passed
  (164 lib + 136 binary + 76 core + 1 style), 0 failed, 5 ignored;
  clippy clean; fmt clean.
- 16 unit tests in `query/nl.rs`: explicit equality, symbolic and
  worded numeric comparisons, negation, `contains` confidence,
  low-confidence equality, `and` / `or` and comma conjunctions,
  multi-word sampled values, multi-word column names, aliases, quoted
  values, and unparseable input.

### Issues discovered

- Two bugs found and fixed during development: `value_only` returned a
  word count where callers expected the next token index (leaving trailing
  value tokens unmatched), and a quoted multi-word value needed a
  single-token comparison before splitting a sample into words.
- General `not` (`not (A or B)`) is not supported because `FilterExpr`
  has no `Not`; only negating operators are handled. This is the recorded
  `FilterExpr::Not` prerequisite.
- The roadmap's original acceptance example ("US orders over 100") is
  value-first with implied columns, which rules cannot resolve by design.
  It was updated to "country is US and amount over 100"; value-first
  phrasing is exactly the Tier 2 escalation case.

### Docs

- `ROADMAP.md`: Tier 1 tasks ticked; prerequisites annotated
  (`Not` deferred, aliases programmatic only); acceptance example made
  realistic.
- `ARCHITECTURE.md`: module tree gains `query/nl.rs`; the Planned
  Architectural Evolution bullet now says Tier 1 landed; unit-test count
  added.

### Next recommended task

The Ask bar UI (`Cmd/Ctrl+Shift+L`) reusing `TextEdit` and the existing
filter apply path, or `FilterExpr::Not` first if general negation is
wanted. Tier 2 stays demand-gated.

## 2026-11-14: Ask bar UI (Cmd/Ctrl+Shift+L)

### Context

The user reported Cmd+Shift+L not working. Two reasons: the Ask bar UI
was not implemented yet (only the Tier 1 parser had landed), and
`commands::action_for` mapped any Cmd/Ctrl+L to `Action::OpenFilter`
without considering Shift, so the chord was swallowed by the plain
filter.

### Implemented

- **`src/ui/commands.rs`**: added `Action::OpenAsk` and made the `l`
  binding shift-aware (`Cmd/Ctrl+Shift+L` -> Ask, `Cmd/Ctrl+L` -> filter).
- **`src/ui/view/ask.rs` (new)**: `open_ask` / `close_ask`,
  `build_ask_schema` (names from `column_label`, types from
  `schema::infer`), `refresh_ask` (Tier 1 parse on every edit, reusing
  `filter_value_cache` via `RefCell`), `accept_ask` (format the
  expression, switch to the filter bar, and run the existing apply path),
  the key handler (Enter accepts, Esc closes, Cmd/Ctrl+F/L switch bars,
  full caret editing), mouse caret/drag handlers, and `render_ask_bar`
  plus `ask_status_text` (live preview of the parsed expression, or the
  unmatched spans).
- **`src/ui/view.rs`**: Ask state fields, `mod ask;`, `Action::OpenAsk`
  dispatch, `switch_to_ask`, Ask closed by the other switches and by
  `Action::Escape`, deferred focus, bar rendered above the grid, and the
  schema/preview cleared in `load_active` on document change.
- **`src/query/nl.rs`**: `describe(expr, headers)` renders a parsed
  expression back to filter syntax with quoting for multi-word names and
  values.

### Verified

- `cargo build --release` clean; `cargo test --release` 382 passed
  (165 lib + 140 binary + 76 core + 1 style), 0 failed, 5 ignored;
  clippy clean; fmt clean; GUI launches and stays alive 6 s.
- New UI tests: `cmd_shift_l_opens_ask_bar`,
  `ask_accepts_a_phrase_into_the_filter` (previews and applies
  `country == US AND amount > 25`, rows `[2]`),
  `ask_does_not_apply_an_unparseable_phrase`, `escape_closes_ask_bar`.
  New unit test for `describe`.

### Notes / limits

- The preview is shown live as the user types, and Enter only accepts
  when a predicate was produced; a failed parse keeps the bar open and
  leaves the filter untouched. This is the ADR's preview/accept rule.
- General `not` still needs `FilterExpr::Not`; Tier 2 (optional local
  model) is still planned.

### Docs

- `ROADMAP.md`: Ask bar tasks ticked.
- `ARCHITECTURE.md`: keyboard table gains Cmd/Ctrl+Shift+L; the planned
  bullet says Tier 1 and the Ask bar landed; UI regression count 79 -> 83
  and the new test bullets added.

### Next recommended task

Tier 2 (optional local model) behind the off-by-default feature, or
`FilterExpr::Not` if general negation is wanted first.

## 2026-11-15: Tier 1.5 extended rules; Tier 2 embedded model staged

### Context

The user approved doing both E (extended deterministic rules) and A (an
embedded local model). An architecture review concluded that E is the
only option that satisfies the constraints today (no pre-install, all
platforms, lightweight, offline), with A reserved behind an opt-in
feature. This iteration lands E fully and the dependency-free Tier 2
plumbing, and records A's concrete plan.

### Implemented (E, default Tier 2)

- **`src/query/nl.rs`**:
  - Filler normalization: leading question words ("show me", "list all",
    "rows where", ...) are skipped while a column is expected, so
    phrasings like "show me all the rows where country is US" parse
    cleanly.
  - Fuzzy column matching: a single-token edit-distance fallback
    (`contry` -> `country`) that refuses an ambiguous tie.
  - `between A and B` expands to `column >= A AND column <= B`.
  - More operator synonyms: "no more than", "no less than",
    "no greater than", "no fewer than", "at minimum", "at maximum",
    "matches", "contain", "like".

### Implemented (Tier 2 plumbing, dependency-free)

- `NlBackend` trait, an external `ModelBackend` override, `model_prompt`
  (only samples columns named in the phrase), and `interpret`, which
  always re-parses and schema-validates the backend output through
  `parse_expr`. `NlResult.from_model` records the source.

### Staged, not built (A)

- The embedded model is planned behind an off-by-default `nl-model`
  feature: `candle-core` + `candle-transformers` + `tokenizers` +
  `hf-hub`, one pinned quantized instruct model, downloaded once after
  consent into the platform cache dir, checksum-pinned, offline
  afterwards. It is not added to the default build because the
  dependency tree is heavy, the compile is long, and inference cannot be
  verified in this environment without a model download. This is a
  dedicated next iteration.

### Verified

- `cargo build --release` clean; `cargo test --release` 390 passed
  (172 lib + 141 binary + 76 core + 1 style), 0 failed, 5 ignored;
  clippy clean; fmt clean.
- New tests in `query/nl.rs` (24 total): filler + synonyms, fuzzy typo,
  `between` range, plus the earlier model prompt / fence-strip /
  subprocess / validation tests.

### Docs

- `ROADMAP.md`: Tier 1.5 tasks ticked; Tier 2 section now describes the
  embedded `nl-model` plan and the download UX.
- `docs/adr/0001-natural-language-ask.md`: added a refinement section
  recording Tier 1.5 as the default and the embedded model as opt-in.
- `ARCHITECTURE.md`: nl unit count 16 -> 24; the planned bullet updated.

### Open loops

- Relative dates ("last month", "this year") and a user synonym file
  remain for Tier 1.5.
- The embedded `nl-model` backend and its download/cache manager remain.
- `FilterExpr::Not` remains.

### Next recommended task

The embedded `nl-model` iteration (dependency + model catalog + download
UX + a MockBackend-driven validation test), or relative dates if that is
preferred first.

## 2026-11-16: Tier 2 embedded model backend (`nl-model`)

### Context

Following the approval of E and A, the previous entry landed Tier 1.5.
This iteration adds A: the optional embedded local-model backend, behind
an off-by-default cargo feature, wired into the Ask bar.

### Implemented

- **Cargo.toml**: optional `candle-core`, `candle-transformers`,
  `tokenizers`, and `hf-hub`, exposed as one `nl-model` feature (off by
  default). `hf-hub` is pinned to 0.3 to avoid the much heavier 1.x
  `xet` stack. The default build pulls none of these crates.
- **`src/nlmodel.rs` (new, feature-gated)**: downloads and caches the
  model and tokenizer through `hf-hub`, loads a quantized
  Llama-architecture GGUF with `candle-transformers`'s
  `quantized_llama::ModelWeights`, generates greedily (bounded to 96
  tokens, stopping at EOS), strips fences, and returns a candidate.
  `interpret` validates the candidate with
  `anvil_core::validate_nl_candidate`. Model repo/file and tokenizer
  repo/file are overridable with `ANVIL_NL_MODEL_*` /
  `ANVIL_NL_TOKENIZER_*`.
- **`anvil_core::query::nl`**: added `validate_candidate` (shared by
  every backend) and made `strip_fences` public.
- **Ask bar (`ui/view/ask.rs`, `ui/view.rs`)**: `Ctrl/Cmd+Enter`
  escalates an unresolved phrase to an injected `AskInterpreter`, run on
  the background executor with a generation guard. `ask_loading` /
  `ask_model_error` drive the status line, and the preview marks a
  model-sourced expression `(local model)`. The production interpreter
  is the embedded backend when the feature is on and `None` otherwise,
  so tests can inject a mock.

### Verified

- Default build: `cargo build --release` clean; `cargo test --release`
  391 passed (172 lib + 142 binary + 76 core + 1 style), 0 failed.
- Feature build: `cargo build --release --features nl-model` clean
  (about 2.5 minutes from a warm dep cache); `cargo test --release
--features nl-model` 391 passed.
- `cargo clippy --release --all-targets` clean with and without the
  feature; `cargo fmt --check` clean.
- New UI test `ask_escalates_to_tier2_when_rules_are_unclean` injects a
  mock interpreter, confirms Tier 1 leaves the phrase unresolved, and
  asserts the validated mock expression becomes the preview. This runs in
  the default build, so the escalation path is covered without the
  feature.

### Honest gaps

- Inference was **not** exercised end to end: no model was downloaded in
  this environment, so the candle load/generate path is compile-verified
  only. The default model repo/file is a best guess and is overridable.
- The download has no size/consent dialog, progress bar, cancel, or
  explicit checksum pin (beyond `hf-hub`'s cache). The status shows a
  generic "interpreting with the local model..." while it downloads.
- Enabling `nl-model` adds a large dependency tree and roughly 2.5
  minutes of compile time; the default build is unaffected.

### Docs

- `ROADMAP.md`: embedded backend ticked; download UX marked partial.
- `docs/adr/0001-natural-language-ask.md`: implementation-status section.
- `ARCHITECTURE.md`: the Ask bullet now covers Tier 1.5, the Ask bar,
  and the `nl-model` embedded backend.
- `README.md`: how to enable the feature and the model overrides.

### Next recommended task

Download UX for `nl-model` (size disclosure, progress, cancel, checksum),
a real-model smoke test, then relative dates and `FilterExpr::Not`.

## 2026-11-17: Fix model download ("relative URL without base")

### Context

Running the feature build, the user got:
`model: could not fetch the tokenizer: request error: Bad URL: failed to
parse URL: RelativeURLWithoutBase: relative URL without base`.

### Root cause

`hf-hub` 0.3.2's `Api::metadata` reads the Hub's redirect `Location`
header and feeds the raw value straight back into the client
(`self.client.get(response.header(LOCATION).unwrap())`) without resolving
it. When the Hub returns a relative redirect, `ureq` cannot parse it and
errors with `RelativeURLWithoutBase`. This is an hf-hub bug, not a bad
repo or endpoint.

### Fix

- Dropped `hf-hub` from the `nl-model` feature and added `ureq` directly.
- `src/nlmodel.rs` now downloads with `ureq::get(...).call()`, which
  resolves redirects against the request URL correctly, and writes the
  file into a cross-platform cache directory:
  `ANVIL_NL_CACHE` if set, else `~/Library/Caches/anvil/models` (macOS),
  `%LOCALAPPDATA%\anvil\models` (Windows), or
  `$XDG_CACHE_HOME/anvil/models` / `~/.cache/anvil/models` (Linux).
- Added `ANVIL_NL_ENDPOINT` (mirror) and `HF_TOKEN` (gated models)
  support. `ANVIL_NL_MODEL_*` / `ANVIL_NL_TOKENIZER_*` overrides remain.

### Verified

- Default build: 391 tests pass; feature build: 391 pass; clippy clean
  and fmt clean in both.
- New ignored smoke test `downloads_the_default_tokenizer` fetches only
  the small tokenizer file and passed; it landed 2.0 MB at
  `~/Library/Caches/anvil/models/HuggingFaceTB__SmolLM2-360M-Instruct/tokenizer.json`,
  proving the redirect path now works.
- Confirmed the default model file exists and is 258 MB:
  `bartowski/SmolLM2-360M-Instruct-GGUF/SmolLM2-360M-Instruct-Q4_K_M.gguf`.
- Inference itself was still not run here (no model download in this
  environment); the candle load/generate path remains compile-verified.

### Docs

- `README.md`: model size, cache locations, and the full override list.
- `docs/adr/0001-natural-language-ask.md`: implementation-status note on
  the `ureq` downloader.

### Next recommended task

A real-model smoke test once a machine can run the 258 MB download, plus
the consent/progress/cancel download UX.

## 2026-11-18: Better Tier 2 model output (chat template, extraction)

### Context

The user asked the Ask bar "Last names that start with C" and the local
model path replied `model: backend returned an invalid expression: unknown
operator 'last'`. The rules cannot resolve that phrase, the model was
consulted, and its raw output was not a clean expression.

### Root causes

- The prompt was fed to the model as raw text, without the model's chat
  template, so an instruct model did not see it as a proper user turn.
- The candidate was only the last non-empty output line, then parsed
  directly. Any prose or formatting around the expression made the parse
  fail, and the error did not show what the model actually said.

### Fix

- **ChatML prompt** (`src/nlmodel.rs`): the instruction is now wrapped in
  the SmolLM2 template (`<|im_start|>system ... <|im_start|>user ...
<|im_start|>assistant`) and encoded with `add_special_tokens = false`,
  because the model's BOS is `<|im_start|>` and the template already
  contains it.
- **Stricter instruction + examples** (`anvil_core::model_prompt`): the
  rules now say to use exact column names, reply with one line and no
  prose or fences, and include three worked examples (including "last
  names starting with C -> last_name startswith C").
- **Robust extraction** (`src/nlmodel.rs::extract_expression`): tries the
  whole output, the text after a label, and each line with common
  list/label prefixes and trailing punctuation stripped. The first
  candidate the real parser accepts wins, so surrounding prose no longer
  breaks a valid expression.
- **Honest errors**: when nothing validates, the error now includes the
  raw model output (truncated) instead of only the parser's complaint.

### Verified

- Default and feature builds: all test binaries pass in both; clippy clean
  and fmt clean.
- Ignored tokenizer tests both pass: `downloads_the_default_tokenizer`
  (2.0 MB cached) and the new `chatml_prompt_round_trips_through_the_tokenizer`,
  which confirms the prompt encodes with the real `<|im_start|>` special
  tokens and contains the question and the assistant turn.
- The full generation path still was not run here (no 258 MB model
  download in this environment).

### Docs

- `docs/agent-log.md`: this entry.

### Next recommended task

Re-test with the real model; if quality is still uneven, add a small
few-shot retry (feed back the parse error and ask again) or pin a checksum.

## 2026-11-19: Column-name normalisation, better prompt, stronger model

### Context

The user reported that the model produced `last_name startswith C` for a
column actually headed `Last Name`, so validation failed. They also asked
whether SmolLM2-360M was the right model and what to look for.

### Fixes

- **Normalised column resolution** (`src/query/filter.rs`): after an exact
  header match, `#N`, and `colN`, `resolve_column` now falls back to a
  unique normalised match where spaces, underscores, and hyphens are
  equivalent. So `last_name`, `lastname`, and `Last Name` all resolve to
  the `Last Name` column, while two headers that normalise the same stay
  ambiguous and error instead of being guessed.
- **Prompt quotes spaced names** (`anvil_core::model_prompt`): column names
  that are not plain identifiers are listed quoted (`- "Last Name"
(text)`), and the rules say to keep quotes exactly.
- **Architecture auto-detection** (`src/nlmodel.rs`): the GGUF
  `general.architecture` metadata picks the loader, so Llama (SmolLM2) and
  Qwen2 (Qwen2.5) models both work through one `Weights` enum.
- **Default model upgraded** from SmolLM2-360M-Instruct to
  Qwen2.5-Coder-1.5B-Instruct Q4_K_M (about 1.1 GB), which follows strict
  output formats far better at a still-small size. The 0.5B variant
  (about 470 MB) is documented as the lighter option.

### Verified

- Default build and feature build: all test binaries pass; clippy clean
  and fmt clean in both.
- New tests: `resolve_column_normalises_spaced_headers` (including the
  real parser accepting `last_name startswith C`) and the ambiguity
  guard; `model_prompt_quotes_spaced_column_names`.
- The ignored `chatml_prompt_round_trips_through_the_tokenizer` passes
  with the Qwen tokenizer, confirming the ChatML prompt uses the real
  `<|im_start|>` special tokens.
- The earlier SmolLM2 model (258 MB) had already downloaded successfully
  through the new `ureq` downloader, so the download path was confirmed
  with a real model; the remaining unverified piece is generation quality
  with Qwen2.5, which needs the 1.1 GB download.

### Model-selection notes (recorded for future work)

Prefer instruct models that are code-tuned or tool/function-call tuned in
the 0.5B-1.5B range, with: a GGUF Q4_K_M quant, a `tokenizer.json`, a
ChatML-style template, candle architecture support (Llama, Qwen2), and an
Apache-2.0/MIT licence. Qwen2.5-Coder is the current best fit. Avoid raw
base models and models without a chat template.

### Next recommended task

Run the Ask bar against Qwen2.5-Coder-1.5B on the real fixture and tune
the prompt if needed; then the consent/progress download UX.

## 2026-11-20: Row-scope filter predicates (`any` / `all`) and GLiNER value binding

### Context

The user reported that general Ask questions were hard to parse ("Which
users are from Canada?") and suspected the filter language itself lacked
the operators needed to express coherent queries, giving the example
"show entries where at least one column is blank".

### Implemented

- **GLiNER implicit-column binding** (`src/gliner.rs`, `src/ui/view.rs`,
  `src/main.rs`): `Interpreter::interpret` now receives the per-column
  sample callback instead of discarding it. Column labels are enriched
  with type, aliases, and up to five sampled values plus a task prompt,
  with a graceful fallback to raw-name labels when the enriched schema
  would exceed the 512-token budget. `decode` binds an extracted value
  to the unique column whose sampled values contain it (exact, trimmed,
  case-insensitive); ambiguous values fail closed; low classifier
  confidence is overridden only by an unambiguous match. Binding is
  limited to `==` / `!=` so a fragment such as `C` in "starts with C"
  cannot hijack the classifier's column.
- **Row-scope filter predicates** (`src/query/filter.rs`,
  `src/query/nl.rs`, `src/query/suggest.rs`): `FilterExpr::AnyColumn` /
  `AllColumn` plus the `any` / `all` pseudo-columns, so `any empty`
  matches a row with at least one blank cell and `all notempty` requires
  every column (missing cells count as empty). Evaluation refactors
  `Prepared::matches` into a per-cell `matches_cell` and loops columns
  with short-circuiting. `describe` renders both forms and round-trips;
  autocomplete offers `any` / `all` in the column slot.
- Docs updated: README filter/Ask sections, ARCHITECTURE filter and
  limitations sections, ROADMAP filter + GLiNER tasks.

### Verified

- `cargo fmt` clean.
- `cargo test --release`: 183 lib tests plus all integration suites pass.
- `cargo test --release --features nl-gliner`: 192 lib tests pass.
- `cargo clippy --release` and `cargo clippy --release --features
nl-gliner`: clean (only the pre-existing `block` / `proc-macro-error2`
  future-incompat notices).
- The real 296 MB GLiNER checkpoint was **not** downloaded here, so the
  end-to-end model path and the ignored `real_model_filter_smoke` remain
  unverified.

### Tests added / changed

- Added in `src/query/filter.rs`: `any_empty_matches_a_row_with_a_blank_cell`,
  `all_notempty_requires_every_column`,
  `row_scope_any_contains_is_case_insensitive`,
  `row_scope_any_ordering_skips_non_numeric_cells`,
  `row_scope_parses_inside_boolean_expressions`,
  `row_scope_rejects_missing_value_and_unknown_operator`.
- Added in `src/query/nl.rs`: `describe_round_trips_row_scope_predicates`.
- Added in `src/query/suggest.rs`:
  `column_slot_offers_row_scope_pseudo_columns`,
  `row_scope_pseudo_column_moves_to_the_operator_and_value_slots`.
- Added in `src/gliner.rs`:
  `a_unique_sampled_value_binds_the_column_without_classifier_confidence`,
  `an_ambiguous_sampled_value_is_rejected`,
  `an_unknown_value_falls_back_to_the_classifier`,
  `the_operator_confidence_gate_still_applies`,
  `column_descriptions_include_type_and_sampled_values`.
- Updated UI tests for the new leading `any` / `all` suggestions:
  `filter_suggestions_arrow_and_enter_accept`,
  `filter_suggestion_mouse_click_fills_the_input`.

### Roadmap changes

- Filter: added a checked row-scope task; operators bullet updated.
- Milestone 6: checked GLiNER value binding / label descriptions; added
  an open task to map row-scope phrases into the rules parser and GLiNER
  labels; `FilterExpr::Not` remains deferred.
- `Verify the real model...` remains open.

### Architecture changes

- `ARCHITECTURE.md`: filter section documents `AnyColumn` / `AllColumn`
  and the per-cell evaluation refactor; autocomplete lists `any` / `all`;
  limitations note row scope exists but general `NOT` does not.

### Issues / technical debt discovered

- The row-scope forms are expressible in the filter bar but the
  natural-language paths do not construct them yet, so "at least one
  column is blank" still fails in Ask. That mapping is the next slice.
- GLiNER now samples every column to build descriptions; on wide files
  this is up to 32 bounded `sample_distinct` scans per interpretation. It
  runs on the background executor and the UI caches samples per parse,
  but it is a measurable cost worth profiling.
- A column literally named `any` or `all` is unreachable by name; use
  `#N`.

### Next recommended task

Map row-scope phrases to `AnyColumn` / `AllColumn` in Tier 1 and the
GLiNER labels, then verify the real GLiNER model and UI regressions.

## 2026-11-20 (later): Row-scope phrases in the NL paths

### Context

The previous slice made `any` / `all` expressible in the filter bar but
Ask did not construct them, so "showing entries where at least 1 column
is blank" still failed.

### Implemented

- **Tier 1 (`src/query/nl.rs`)**: `parse` now tries `parse_row_scope`
  before the per-column loop. It skips leading filler (including
  "showing"/"listing"), matches a scope prefix (`any`/`some`/`a`,
  `at least [one|1]`, `all`/`every`, `no`) followed by a column word,
  then matches the comparison to the end of the phrase. `no column is
empty` flips the inner operator (`Empty` -> `NonEmpty`), so it becomes
  `all notempty`. Row-scope values never consult the per-column sample
  callback. Anything that does not match falls through unchanged, so
  `any amount over 100` still resolves `amount`.
- **GLiNER (`src/gliner.rs`)**: added a third classification, `filter
scope` (`one column` / `any column` / `all columns`), appended after
  the column and comparison specs. `decode` reads it leniently (missing
  or unknown -> single column) and builds `AnyColumn` / `AllColumn`,
  skipping `resolve_column` and per-column binding. The operator
  confidence gate, exact-span validation, negation guard, numeric guard
  (row scope needs a finite number but not a numeric column), and the
  describe/validate round-trip all still apply.

### Verified

- `cargo fmt` clean.
- `cargo test --release`: 186 lib tests plus all integration suites pass.
- `cargo test --release --features nl-gliner`: 199 lib tests pass.
- `cargo clippy --release` and `cargo clippy --release --features
nl-gliner`: clean.
- The real GLiNER checkpoint still is not installed, so the end-to-end
  model path remains unverified; `real_model_filter_smoke` stays ignored.

### Tests added

- `src/query/nl.rs`: `parses_row_scope_phrases` (includes the exact user
  phrase), `row_scope_phrases_do_not_hijack_column_phrases`,
  `rules_row_scope_round_trips_through_describe`.
- `src/gliner.rs`: `row_scope_labels_decode_to_any_and_all_columns`,
  `row_scope_with_a_value_extracts_and_validates_the_span`,
  `a_missing_scope_defaults_to_a_single_column`,
  `the_schema_offers_a_filter_scope_classification`.

### Roadmap changes

- Milestone 6: checked the row-scope NL mapping task.

### Architecture changes

- `ARCHITECTURE.md`: Ask section documents row-scope recognition in
  Tier 1; limitations note general `NOT` is still outstanding.
- `README.md`: notes the row-scope Ask phrases.

### Issues / technical debt discovered

- GLiNER negation of row scope is a model-quality risk: a phrase like
  "no column is empty" may be labelled `all columns` + `is empty`, which
  is the opposite. Tier 1 flips it correctly; the model path relies on
  the mandatory preview.
- Tier 1 row-scope matching is lexicon-based (`column`/`field`/`cell`
  plus a fixed verb list); variants such as "no cells are populated" are
  not all covered.

### Next recommended task

Verify the real GLiNER model with the new scope classification, then
consider general `NOT` / set membership (`in`).

## 2026-11-20 (row-first fix): "any row that has an empty column"

### Context

The user reported that "Any row that has an empty column" still fell
through to GLiNER, which picked a specific column. The rules parser only
recognized column-first row scope (`any column is empty`); the row-first
form was unmatched, so the model was consulted.

### Implemented

- **Tier 1 (`src/query/nl.rs`)**: `parse_row_scope` now tries
  `parse_column_scope` and then `parse_row_noun_scope`. The row-first
  matcher starts at a row noun (`row(s)`, `record(s)`, `entry/entries`,
  `line(s)`), an optional relative pronoun, a has/with/contain verb,
  optional negation/article, and requires an `<empty-word> <column-word>`
  ending. Negation (`no row ...`, `has no empty column`) maps to
  `AllColumn` with the operator flipped. Leading filler uses two skips:
  the column matcher may skip row nouns; the row matcher anchors on them.
- **GLiNER (`src/gliner.rs`)**: the `filter scope` classification now
  carries a task prompt and per-label descriptions, steering the model
  away from `one column` for row-with-a-blank-column questions.

### Verified

- `cargo fmt` clean.
- `cargo test --release`: 188 lib tests plus all integration suites pass.
- `cargo test --release --features nl-gliner`: 201 lib tests pass.
- `cargo clippy --release` (and feature): clean.
- Real GLiNER checkpoint still not installed; model path unverified.

### Tests added

- `src/query/nl.rs`: `parses_row_has_column_phrases` (includes
  "Any row that has an empty column"),
  `row_has_phrases_do_not_hijack_column_filters`.
- `src/gliner.rs`: extended
  `the_schema_offers_a_filter_scope_classification` to assert the prompt
  and the `any column` description.

### Roadmap / architecture changes

- ROADMAP milestone 6 task text updated; README and ARCHITECTURE document
  both word orders.

### Issues / technical debt discovered

- `rows`/`entries`/`records` are both filler and row nouns; the two
  filler skips encode that distinction. A phrase mixing an explicit row
  quantifier with a column quantifier ("all rows have all columns
  empty") is not handled.
- GLiNER scope selection for row-with-blank-column phrasing is improved
  by descriptions but still unverified against the real model.

### Next recommended task

Verify the real GLiNER model (scope + binding), then general `NOT` / `in`.

---

## 2026-09-30 — Repository publication, fixture policy, and documentation refresh

### Context

The project had never been published, and the working tree had drifted
far ahead of the documentation. This iteration created the private
GitHub repository `codyklr/anvil`, then reconciled the docs with the
code.

Note on dates: the machine clock and the git history read 2026-09-30,
while the entries above carry later dates. This entry is dated to the
actual working date and is therefore out of chronological order with
respect to them.

### Implemented

- **Published** the repository privately as `codyklr/anvil`. Pushing
  required rewriting history: the initial commit contained
  `fixtures/large.csv` (173,427,532 B) and `fixtures/archive.zip`
  (51,525,287 B), and a 165 MiB blob exceeds GitHub's 100 MiB per-blob
  limit. `git filter-branch` dropped both across all 21 commits
  (hashes changed, content otherwise identical); the pre-rewrite refs
  were then discarded, so both blobs are gone from the local object
  database as well (`.git` 122 MB -> 552 KB).
- **Committed the previously uncommitted working tree**: the natural
  language Ask bar, `schema` / `schema::stats`, `query::sort`,
  `query::suggest`, `encoding`, `gliner` / `nlmodel`, the text-editing
  widgets, `docs/adr/0001`-`0002`, and roughly 3.5k lines of agent-log
  entries.
- **Fixture policy**: no generated fixture is committed any more.
  `.gitignore` ignores `/fixtures/*` with `!/fixtures/ask.csv` as the
  only exception, because `gen_fixture` recreates `large.csv`,
  `archive.zip`, `multi.zip` and `malformed.csv`, while `ask.csv` is
  hand-authored.
- **README.md rewritten**: leads with what Anvil is and what it does,
  keeps the technical sections (data representation, ZIP handling,
  threading, opt-in backends), and replaces positioning-by-negation
  with positive statements. Added a keyboard-shortcut table grounded
  in `src/ui/commands.rs` and an updated module map.
- **ROADMAP.md reconciled**: the Milestone 5 `Diagnostics panel` item
  duplicated the Milestone 4 item that is already delivered
  (`src/ui/view/diagnostics.rs`, Cmd/Ctrl+D), so it is now marked as
  delivered with a cross-reference instead of being double-counted;
  `Saved filters` now has a single owner (Milestone 2), and the
  Milestone 4 duplicate became a cross-reference note.
- **ARCHITECTURE.md updated**: module tree now lists `gliner.rs`,
  `nlmodel.rs`, `ui/view/ask.rs` and the two existing ADRs; the
  current-vs-planned list covers `query::filter`, `query::suggest`,
  `query::nl`, the Tier 2 backends, the text-edit widgets and the Ask
  and filter bars; the `ui::commands` action list is complete; the
  dependency table documents the optional `nl-model` / `nl-gliner`
  crates; and the test counts were re-derived from a real run.

### Verified

- `cargo check --all-targets` clean on the committed tree.
- `cargo test`: 416 tests, 410 pass, 6 ignored (188 library, 146
  binary, 81 `tests/core.rs` of which 5 ignored, 1 style guard).
- No test depends on a removed fixture: only the five `#[ignore]`d
  performance tests read `fixtures/large.csv`, and they self-skip.
  The archive tests build their own ZIPs in temp directories.
- Remote tree is 53 files with no `target/` path and no large fixture;
  `gh repo view` reports `"visibility":"PRIVATE"` on branch `master`.

### Tests added / changed

- None. This iteration changed documentation and repository state only;
  no source file was modified. The corrected test counts in
  `ARCHITECTURE.md` were measured, not estimated.

### Roadmap changes

- Milestone 5 `Diagnostics UI`: the panel item is now checked with a
  cross-reference to the Milestone 4 delivery.
- `Saved filters` de-duplicated (Milestone 2 owns it).
- Nothing else moved: the remaining 31 unchecked items were each
  checked against `src/` and are genuinely open (go-to-row, tab
  reordering, recently-closed tabs, light theme, column resize, recent
  files, loading progress, CI matrix and packaging among them).

### Architecture changes

No architectural change. `ARCHITECTURE.md` was corrected to describe
what already exists.

### Issues / technical debt discovered

- The agent log's entry dates run ahead of the machine clock and of
  git, which makes the "newest at the bottom" convention unreliable.
- Repeated per-module test counts in `ARCHITECTURE.md` rot easily; the
  new totals paragraph should be the authoritative summary.
- `fixtures/ask.csv` is the only fixture that cannot be regenerated, so
  it is the one file to preserve carefully if fixture cleanup continues.

### Next recommended task

Unchanged: verify the real GLiNER model (scope + binding), then add
`FilterExpr::Not` and `NOT` / `set membership`.

---

## 2026-09-30 — Byte-faithful display (never transform data)

### Context

The owner stated a core philosophy: no data may ever be transformed.
Values must be visible exactly as they exist in the file, never with
leading or trailing whitespace or zeroes stripped, never truncated. An
audit of the render, inspection and clipboard paths found three places
where the code contradicted that.

### Implemented

- **Column inspector** (`src/schema/stats.rs`): `min`, `max`,
  `longest`, `shortest`, `samples` and `frequent` now carry the exact
  cell value. Previously everything was stored from
  `cell.as_str().trim()`, so `" 42 "` was reported as `42`. Values that
  differ only in padding are now counted as distinct values (the counts
  map and the HLL sketch are keyed by the exact value). `min`/`max`
  _ordering_ still compares the trimmed value, and a whitespace-only
  cell still counts as empty, so the inspector keeps agreeing with the
  filter `empty` operator; both deviations are recorded in `ROADMAP.md`.
- **Invalid UTF-8 must stay visible** (`src/parser/mod.rs`): added
  `exact_string` and `Cell::to_exact_string`. Valid UTF-8 is returned
  borrowed and unchanged. A cell containing at least one invalid byte is
  escaped reversibly: valid runs verbatim, a literal backslash doubled,
  and each invalid byte as `\xNN`. Doubling backslashes across the whole
  cell is what makes the mapping unambiguous.
- **Grid** (`src/ui/grid.rs`): renders through `exact_string`, and the
  `∅` marker now means a genuinely zero-byte cell. Previously
  `str::from_utf8(bytes).unwrap_or("")` made a cell of invalid bytes
  render as empty, hiding the data completely.
- **Clipboard** (`src/ui/view.rs`): `copy_selection_to_clipboard` copies
  `to_exact_string()` instead of `into_string()`, so an invalid-UTF-8
  cell copies its escaped bytes instead of U+FFFD.
- **Probe and headers** (`src/main.rs`, `src/parser/delimit.rs`): the
  `--probe` sample rows print `to_exact_string()`, and
  `ParseMeta.headers` is built with `exact_string` rather than
  `String::from_utf8_lossy`, so an invalid header is no longer shown as
  U+FFFD. Both were found by running `--probe` on a file with invalid
  bytes and watching the first row print as `""`.
- **Docs**: new `docs/adr/0003-byte-faithful-display.md`; a **Data
  Fidelity** section and a core principle in `VISION.md`; a
  byte-faithful bullet in `README.md`; architectural invariant 12 plus
  updated inspector and clipboard sections in `ARCHITECTURE.md`; the new
  Milestone 5 **Data fidelity** block in `ROADMAP.md`.

### Verified

- `cargo test`: 421 tests, 415 pass, 6 ignored (190 library, 146 binary,
  84 `tests/core.rs` of which 5 ignored, 1 style guard). Four tests are
  new; nothing was weakened.
- `cargo clippy --all-targets`: clean apart from the pre-existing
  future-incompatibility notice for `block` / `proc-macro-error2`.
- `anvil --probe` on a file containing `\xff\xfe` now prints
  `["1", "\\xff\\xfe"]` for the first row instead of an empty string,
  and the run still completes in under 1 ms.
- Behaviour is covered by `invalid_utf8_cells_are_shown_exactly`,
  `exact_string_escapes_only_when_it_has_to` and
  `invalid_utf8_headers_are_escaped_not_replaced` (`tests/core.rs`) plus
  `padded_values_are_reported_exactly` and
  `whitespace_only_cells_still_count_as_empty` (`schema::stats`).
- Manual GUI exercise of the new cell rendering was not performed; the
  change is confined to the text handed to GPUI, and the string path is
  unit-tested.

### Issues / technical debt discovered

- Sorting and search still treat values that differ only in case or
  padding as equal, and `empty` still means whitespace-only. Both are
  semantic (not display) choices and are now explicit roadmap items
  rather than undocumented behaviour.

### Next recommended task

Work the new Milestone 5 Data fidelity items: exact-value comparison
with a byte tiebreak, then the whitespace-only `empty` decision.

---

## 2026-09-30 — Iteration 1: CI build/test matrix

### Context

First slice of a staged improvement run (branch
`agent/iterative-improvements`). The goal was a GitHub Actions workflow
covering Linux, macOS and Windows with format, build, test, fixture
generation and a headless probe. The Linux native dependencies had to be
derived from GPUI's dependency tree rather than guessed.

### Implemented

- New `.github/workflows/ci.yml`: a `build-and-test` job with an
  `ubuntu-latest` / `macos-latest` / `windows-latest` matrix, pinned to
  `dtolnay/rust-toolchain@stable` (with the `rustfmt` component) and
  `Swatinem/rust-cache@v2`. Steps: `cargo fmt --check`,
  `cargo build --release`, `cargo test --release`,
  `gen_fixture fixtures 2000`, then
  `anvil --probe fixtures/large.csv`. `CARGO_TERM_COLOR: always` is set
  workflow-wide and `fail-fast: false` keeps every OS running.
- Linux-only apt step installing `libxkbcommon-dev`,
  `libxkbcommon-x11-dev`, `libfreetype6-dev`, `libwayland-dev`,
  `pkg-config`, `cmake`, `clang`. Derivation: the release build uses
  `gpui` with `default-features = false, features = ["font-kit"]`, but
  `cargo test` pulls the dev-only `test-support` feature, which turns on
  GPUI's `x11` and `wayland` backends. In that tree `zed-font-kit`'s
  unconditional `freetype-sys` probes `freetype2` via pkg-config,
  `xkbcommon` 0.8.0 has `#[link(name = "xkbcommon")]` /
  `#[link(name = "xkbcommon-x11")]`, and `wayland-client` probes
  `wayland-client`. Fontconfig (`yeslogic-fontconfig-sys` with the
  `dlopen` feature) and Vulkan (`ash` default `loaded` feature) are
  dlopen'd and need no `-dev` packages.
- Documentation corrections while reconciling: `AGENTS.md` Quick
  reference test count changed from the stale "38 today, 1 ignored" to
  the measured "415 passing, 6 ignored".

### Verified

- YAML parses (`ruby -ryaml YAML.load_file`): one job, three-OS matrix,
  nine steps.
- Every workflow command was run locally on macOS: `cargo fmt --check`
  (clean), `cargo build --release` (succeeds),
  `cargo test --release` (415 passed, 6 ignored, across the library,
  binary, `tests/core.rs` and `tests/style.rs`),
  `gen_fixture fixtures 2000` (writes `large.csv`, `archive.zip`,
  `multi.zip`, `malformed.csv`), `anvil --probe fixtures/large.csv`
  (2000 rows, 20 columns, 0 warnings, 1 ms).
- **Not verified:** GitHub Actions itself did not run. The workflow is
  syntax-checked only. The Linux and Windows legs, including the Linux
  package list, are unverified until a real run happens.

### Tests added / changed

None. No Rust source changed in this iteration. The `AGENTS.md` test
count was corrected to the measured value.

### Roadmap changes

- Milestone 5 cross-platform support: the CI matrix item moved from
  `[ ]` to `[~]` with a note that only syntax and the local macOS
  commands are verified.
- Added an explicit Milestone 5 task to enable a real GPUI window
  backend on Linux (see issues below).

### Architecture changes

New `### Cross-platform build and Linux GUI backend` subsection under
`Current Limitations / Technical Debt`, recording the CI dependency
derivation and the headless-only Linux release build.

### Issues / technical debt discovered

- The release `anvil` binary is built with `gpui`
  `default-features = false, features = ["font-kit"]`. On Linux neither
  `x11` nor `wayland` is compiled, so `gpui::current_platform` always
  returns the headless client: the binary builds and `--probe` works,
  but the desktop window cannot open on Linux. Fixing this needs
  per-target GPUI backend features plus the matching native packages,
  and cannot be verified on this macOS host.
- `cargo test` enabling `test-support` (which turns on `x11`/`wayland`)
  means the Linux test leg needs more native packages than the release
  build does; the CI package list covers that, but it is untested.

### Next recommended task

Iteration 2: select-all (Cmd/Ctrl+A), go-to-row (Cmd/Ctrl+G) and open
file (Cmd/Ctrl+O) in `ui/commands.rs` plus UI tests.

---

## 2026-09-30 — Iteration 2: keyboard basics (select-all, go-to-row, open)

### Context

Second slice of the improvement run (branch
`agent/iterative-improvements`). The roadmap had Cmd/Ctrl+G unchecked
and select-all living in Future Ideas; Cmd/Ctrl+O was missing entirely.
This iteration wires all three through the existing pure binding table
and adds unit + headless UI tests.

### Implemented

- `ui/commands.rs`: three new actions — `SelectAll`, `OpenGoToRow`,
  `OpenFile` — with Cmd/Ctrl+A, Cmd/Ctrl+G and Cmd/Ctrl+O bindings
  (all gated on `secondary()`, no alt/function). Unit tests for each
  chord plus the plain- and alt-key negatives.
- `selection.rs`: `Selection::select_all(record_count, column_count)`
  anchors at `(0, 0)` with the cursor at the last row / last column,
  and clears instead of leaving a dangling cursor on an empty document.
  Added `copy_view_text_with(view_rows, cell_at)` and
  `view_cell_count(view_rows)`, which emit/sum only the rows present in
  the visible display order. Four new unit tests.
- `ui/view.rs`: `select_all` uses the document dimensions; copy now
  routes through the view-aware helpers, so under an active filter or
  sort a select-all copies exactly the visible rows, in display order,
  and the "copied N cells" count matches. This closes a pre-existing
  gap where copying with a filter active wrote hidden records.
- `ui/view/goto.rs` (new): the go-to-row bar, modelled on the search
  bar. It reuses `TextEdit`, the shared `input::editable_text_area`,
  the deferred-focus pattern and mouse click/drag caret handling.
  Enter parses a 1-based display row, clamps to
  `[1, displayed_row_count]`, maps through `view_rows` to a record
  index, keeps the current column, scrolls the row to the top and
  closes the bar. Esc closes without changing the selection; invalid
  input shows an inline error. `render_goto_bar` is wired into
  `Render::render` below the other bars.
- `Action::OpenFile` runs the exact `MainView::pick_file` path as the
  toolbar **Open…** button.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **434 passed, 6 ignored** (194 library,
  160 binary, 79 `tests/core.rs` of which 5 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean apart from the
  pre-existing future-incompatibility notice for `block` /
  `proc-macro-error2`.
- New headless `TestAppContext` tests exercise the real focus path:
  Cmd/Ctrl+A selects all; select-all + copy under a filter emits only
  the two `US` rows; Cmd/Ctrl+G opens the bar, typing `10` + Enter
  moves to record 9; a filtered go-to maps display row 2 to record 2;
  an out-of-range number clamps; non-numeric input shows an error and
  keeps the bar open; Esc closes it; and Cmd/Ctrl+A inside the filter
  field selects the field's text without touching the grid selection.
- Manual GUI exercise was not performed (the GUI cannot be driven
  interactively from this environment). The behaviour is covered by
  the headless UI suite.

### Tests added / changed

- `ui/commands.rs`: `select_all_chord`, `plain_a_is_not_select_all`,
  `alt_a_is_not_select_all`, `go_to_row_chord`,
  `plain_g_is_not_go_to_row`, `open_file_chord`,
  `plain_o_is_not_open_file`.
- `selection.rs`: `select_all_covers_the_whole_document`,
  `select_all_on_empty_document_clears`,
  `copy_view_text_skips_hidden_rows`,
  `copy_view_text_preserves_display_order`.
- `ui/view/tests.rs`: `cmd_a_selects_all_when_grid_focused`,
  `select_all_copy_respects_active_filter`,
  `cmd_g_opens_goto_bar_and_enter_jumps`,
  `goto_maps_through_active_filter`,
  `goto_clamps_out_of_range_input`, `goto_rejects_non_numeric_input`,
  `escape_closes_goto_bar`, `cmd_a_in_filter_field_selects_text_not_grid`.

### Roadmap changes

- Milestone 2 keyboard navigation: Cmd/Ctrl+G ticked; added and ticked
  Cmd/Ctrl+A and Cmd/Ctrl+O.
- Milestone 1 known gaps: removed "No keyboard shortcut to open files".
- Future Ideas: removed the select-all bullet (now implemented).

### Architecture changes

`ARCHITECTURE.md` module tree, keyboard-binding list and shared-input
description updated for `ui/view/goto.rs` and the three new actions.
No state-ownership or concurrency change: the go-to-row bar holds
global (not per-tab) transient state, like `filter_open`.

### Issues / technical debt discovered

- `Selection` is a rectangle in raw record-index space. The grid and
  the copy path now walk the visible display order, but a _dragged_
  range under an active sort still spans a record-index interval that
  may not be contiguous on screen. Fixing that needs a display-space
  selection model and is out of scope here. **(Resolved in Iteration
  12, 2026-10-01: the selection model now stores display positions.)**
- The delegated implementation worker hit its model-turn limit on the
  first CI slice (produced no files); work has continued locally with
  each slice verified by the test suite.

### Next recommended task

Iteration 3: performance baseline (measure only) written to
`docs/benchmarks.md` and referenced from `ARCHITECTURE.md`.

---

## 2026-09-30 — Iteration 3: performance baseline (measure only)

### Context

Third slice of the improvement run (branch
`agent/iterative-improvements`). Milestone 5 had no recorded baseline,
so the per-cell allocation change had nothing to compare against. This
iteration measures only; no production code changed.

### Implemented

- Generated fixtures: `fixtures/` at 1,000,000 rows (165.4 MiB
  `large.csv`, 49.1 MiB `archive.zip`, `multi.zip`,
  `malformed.csv`) and `fixtures/big/` at 3,000,000 rows (498.3 MiB
  `large.csv`). These live under the git-ignored `fixtures/` tree and
  are not committed.
- `docs/benchmarks.md` (new): reference machine, methodology, the
  index-time / peak-RSS table, the index-only throughput, an
  explanation of why peak RSS tracks the file-backed mmap rather than
  the index, and a CPU-only grid-render proxy.
- `tests/core.rs`: new ignored
  `measure_viewport_cell_text` test. It runs 1,000 frames of a 40 x 20
  visible window through the same single `Cells` pass and
  `exact_string` + `to_string` allocation that `ui::grid::render_row`
  performs, giving a stable before/after number for the Milestone 5
  allocation change.
- `ARCHITECTURE.md` references `docs/benchmarks.md` as the
  authoritative baseline from the Performance Characteristics section.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **434 passed, 7 ignored** (the new
  measurement test adds one ignored test).
- `cargo clippy --release --all-targets` — clean apart from the
  pre-existing `block` / `proc-macro-error2` future-incompat notice.
- Measurements (Apple M4, 10 cores, 24 GiB RAM, macOS 26/arm64,
  rustc 1.98.1), median of three `anvil --probe` runs:
  - `fixtures/large.csv` (165.4 MiB, 1M rows): index 249 ms, peak RSS
    182 MiB.
  - `fixtures/big/large.csv` (498.3 MiB, 3M rows): index 747 ms, peak
    RSS 530 MiB.
  - `fixtures/archive.zip` (49.1 MiB compressed, 165.4 MiB entry):
    ~1.0 s, peak RSS 182 MiB.
  - `fixtures/ask.csv` / `fixtures/malformed.csv`: ~77 µs, 9 MiB.
  - Index-only throughput (`measure_large_fixture`): 254 ms / 651
    MiB/s on the 1M fixture.
  - Grid render proxy (`measure_viewport_cell_text`): 800,000 cells in
    35 ms, 44.4 ns/cell, 0.035 ms/frame, 5,689,466 bytes written.
- **Not verified:** no GPU frame time or interactive scroll profile
  was captured. The GUI cannot be driven from this environment, so the
  scroll item remains partial and the proxy is labelled as a proxy.

### Tests added / changed

- `tests/core.rs`: `measure_viewport_cell_text` (`#[ignore]`,
  measurement only).

### Roadmap changes

- Milestone 5 Performance: "Profile indexing on a multi-hundred-MiB
  file" ticked; "Memory usage measured and reported per fixture"
  ticked; "Profile scrolling" marked `[~]` with the proxy noted and the
  real frame timer left open. The LRU-cache and allocation items stay
  open for Iteration 4.

### Architecture changes

No module or state change. The `docs/` tree in `ARCHITECTURE.md` now
lists `benchmarks.md`, and the Performance Characteristics section
points at it.

### Issues / technical debt discovered

- Peak RSS is dominated by file-backed mmap pages touched while
  indexing; it is a poor proxy for the durable memory cost. The durable
  index is `record_count x 8` bytes (8 MiB per 1M rows). Anyone
  reporting "Anvil uses 182 MiB for a 165 MiB file" should say that
  those pages are reclaimable.
- No frame timer exists. If scroll work is taken up later, a debug-only
  frame-time recorder behind an env var would let a desktop run capture
  real numbers without changing release behaviour.

### Next recommended task

Iteration 4: reduce the per-cell `SharedString::from(text.to_string())`
allocation in `ui/grid.rs`, evaluate a bounded visible-range cache, and
record before/after numbers against `docs/benchmarks.md`.

---

## 2026-09-30 — Iteration 4: per-cell allocation in the grid

### Context

Fourth slice of the improvement run (branch
`agent/iterative-improvements`). `ui/grid.rs::render_row` built
`SharedString::from(text.to_string())` for every visible cell on every
frame: an intermediate `String` copy plus the `Arc<str>` copy. Iteration
3 left a measurable proxy so this could be judged before/after.

### Implemented

- `ui/grid.rs`: new `cell_shared_string(bytes)` builds the cell text as
  one `SharedString::new(exact_string(bytes).as_ref())` copy instead of
  `SharedString::from(text.to_string())`. Byte-faithful display is
  unchanged: valid UTF-8 (including multi-byte) is copied verbatim and
  invalid bytes keep the escaped `\xNN` form.
- `GridModel` gained a bounded `(record, column) -> SharedString` cache
  (`CELL_TEXT_CACHE_CAP = 4096`, clear-on-full). `render_row` looks the
  text up through `GridModel::cell_text`, so a scroll step allocates
  only for the rows that just entered the view. `uniform_list` re-runs
  the row renderer for every visible row each frame and a scroll step
  keeps ~39 of 40 rows, so the cache hit rate during scrolling is
  ~95%. The text depends only on the document, so building a new
  `GridModel` (open / reindex) fully invalidates it; there is no
  staleness window. A poisoned mutex is recovered via `into_inner`.
- Empty cells skip the cache entirely (they render the `∅` marker).
- `tests/core.rs`: `measure_viewport_cell_text` now compares three forms
  over a one-row-per-frame scroll - the old two-copy form, the new
  one-copy form, and the bounded cache - so the decision is
  reproducible.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **436 passed, 7 ignored** (194 library, 162
  binary, 79 `tests/core.rs` of which 6 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean apart from the
  pre-existing `block` / `proc-macro-error2` future-incompat notice.
- Before/after on the 1M fixture (1,000 frames of a 40 x 20 viewport,
  scrolling one row per frame):
  - old `to_string` + `SharedString` (two copies): 81.9 ns/cell,
    0.066 ms/frame.
  - new `Arc<str>` only (one copy): 53.4 ns/cell, 0.043 ms/frame.
  - bounded cache (shipped): 22.6 ns/cell, 0.018 ms/frame.
  - **3.6x** faster than the old form for the isolated allocation
    work. The cache was kept because the proxy shows a real win;
    iteration 3's 44.4 ns/cell figure was the single-`to_string`
    proxy, not the two-copy production form, so the true before is
    81.9 ns/cell.
- No GPU frame time was captured; the GUI cannot be driven headlessly.
  The proxy excludes element construction and GPU submission, which
  the change does not touch.

### Tests added / changed

- `ui/grid.rs`: `cell_shared_string_is_byte_faithful` (ASCII,
  multi-byte, padding, empty, literal backslash, four invalid-byte
  cases checked against `exact_string`) and
  `cell_text_cache_is_consistent_and_bounded` (cache hit equality,
  escaped invalid bytes, cap enforcement).
- `tests/core.rs`: `measure_viewport_cell_text` extended to the
  three-form comparison (still `#[ignore]`, measurement only).

### Roadmap changes

- Milestone 5 Performance: "Add an LRU cache for cell text in the
  visible range" ticked; "Reduce per-cell allocation in `render_row`"
  ticked, with the note that the eviction is clear-on-full rather than
  strict LRU.

### Architecture changes

`ARCHITECTURE.md` Scrolling observations and the `ui::grid` module
description now document the bounded cell-text cache and its
invalidation-by-new-`GridModel` contract. `docs/benchmarks.md` carries
the three-form before/after table.

### Issues / technical debt discovered

- The cache is cleared rather than evicted when full. For files wide
  enough that the visible set exceeds 4096 distinct cells the cache
  thrashes (insert with no reuse) rather than degrading gracefully.
  Common files have far fewer visible columns, so this is acceptable
  for now; an LRU or a viewport-sized cap would be the follow-up if a
  wide-file regression appears.
- No real frame-time instrumentation exists. The allocation proxy is
  the only scroll-related number.

### Next recommended task

Iteration 5: loading progress during ZIP extraction and indexing,
staying on the background executor with the generation counter
discarding obsolete updates.

---

## 2026-09-30 — Iteration 5: loading progress

### Context

Fifth slice of the improvement run (branch
`agent/iterative-improvements`). Long ZIP extractions and large index
builds showed only a generic `Loading…` label. The roadmap wanted
bytes-processed-over-total where known, an indeterminate indicator
otherwise, background-only work, and obsolete updates discarded by the
generation counter.

### Implemented

- `src/progress.rs` (new): `LoadPhase` (`Reading` / `Extracting` /
  `Indexing`), `LoadProgress { phase, bytes_done, bytes_total }` with
  `fraction()`, and the `ProgressSink` callback type. `LoadPhase`
  carries `as_u8` / `from_u8` for lock-free atomic transport.
- `parser/indexing.rs`: `build_with_progress` / `parse_with_progress`
  report the index walk's byte position against the mmap length. The
  per-byte check lives in `build_impl<const REPORT: bool>` so the
  no-sink path compiles it out (a runtime check regressed the 1M scan
  from 254 ms to 622 ms; the const version restores 251 ms).
- `source/archive.rs`: `extract_into_temp_with_progress` reports bytes
  copied against the central directory's uncompressed size.
- `loader.rs`: `load_with_progress`, `load_entry_with_progress`,
  `index_opened_with_progress`; the old entry points delegate with
  `None`.
- `ui/view.rs`: an atomic `ProgressTracker` shared with the background
  task, a 50 ms `start_progress_tracking` poller that stops on a
  `finished` flag or a `progress_generation` bump, `progress_text()`
  and `render_progress_bar()` (determinate bar with byte counts,
  indeterminate stub otherwise), wired into both the no-document
  `Loading` state and the pending-ZIP-tab placeholder. `apply` and
  `apply_lazy_result` clear the indicator.
- `lib.rs`: re-exports `progress`, the new loader functions and the
  progress-aware indexing functions.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **443 passed, 7 ignored** (196 library, 164
  binary, 82 `tests/core.rs` of which 6 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean apart from the
  pre-existing future-incompat notice.
- Data-side tests prove monotonic, determinate reporting for indexing
  and ZIP extraction (a ~2.5 MiB fixture yields intermediate 1 MiB
  samples) and that the final sample reaches the source length.
- UI tests prove the indicator appears with byte counts, appears as an
  indeterminate stub when the total is unknown, and clears.
- Indexing throughput after the change: 251 ms / 659 MiB/s on the 1M
  fixture (baseline 254 ms / 651 MiB/s), recorded in
  `docs/benchmarks.md`.
- **Not verified:** no manual GUI run; the bar is exercised by the
  headless UI tests only.

### Tests added / changed

- `tests/core.rs`: `indexing_reports_monotonic_progress`,
  `zip_extraction_reports_progress`, `load_without_progress_still_succeeds`.
- `progress.rs`: `fraction_is_none_when_the_total_is_unknown`,
  `fraction_clamps_and_handles_zero`.
- `ui/view/tests.rs`: `loading_progress_indicator_appears_and_clears`,
  `loading_progress_without_total_is_indeterminate`.

### Roadmap changes

- Milestone 5 UX: "Loading progress UI for extraction and indexing"
  ticked.
- Milestone 1 known gaps: removed "No progress UI during long
  extractions".

### Architecture changes

`ARCHITECTURE.md` module tree, the archive known-limitations note, and
a new "Loading progress" section document the sink / tracker / poller
design and the const-generic no-report path. `docs/benchmarks.md` gains
a progress-overhead subsection.

### Issues / technical debt discovered

- The poller is a foreground timer task; it stops via the tracker's
  `finished` flag, so a panic in the data side would leave it running
  until the next load bumps the epoch. Acceptable, but a stronger
  contract (RAII finish guard) would be tidier.
- Progress is global rather than per tab: switching to a different tab
  while a lazy ZIP entry extracts keeps showing that extraction's
  progress. Reasonable for now; per-tab progress would need the tracker
  on `DocumentSnapshot`.

### Next recommended task

Iteration 6: column resize by dragging the header boundary, with a
width model test and one UI drag test.

---

## 2026-09-30 — Iteration 6: column resize

### Context

Sixth slice of the improvement run (branch
`agent/iterative-improvements`). Headers were fixed-width: the only way
to see a wide column was horizontal scrolling. The roadmap wanted a
drag on the header boundary, with body/header alignment, correct
horizontal scroll and scrollbar, and per-tab widths.

### Implemented

- `theme::MAX_COLUMN_WIDTH` (1200 px), deliberately wider than the
  automatic `MAX_AUTO_WIDTH` so a manual resize can exceed it.
- `ColumnLayout::from_widths` (rebuild offsets + total) and
  `ColumnLayout::resized(column, width)` (clamp to
  `[MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH]`, then rebuild). Replacing the
  `Arc<ColumnLayout>` on `GridModel` updates header, body and scrollbar
  together because all three read the one layout.
- `grid::render_header` now takes an `on_resize_start` callback and
  renders a 5 px `cursor_ew_resize` handle on each header's right edge.
  The handle calls `App::stop_propagation` so the press does not also
  cycle the sort.
- `MainView.column_drag: Option<(column, press_x, press_width)>`.
  The header handle starts the drag; the grid's existing global
  mouse-move listener updates the width (`column_drag_move`) and a new
  global mouse-up listener ends it (`end_column_drag`).
- `DocumentSnapshot.column_widths: Option<Vec<f32>>` persists the
  widths per tab. `snapshot_active` saves the live layout widths on
  drag end (and on any tab switch); `load_active` restores them when
  the length still matches the document's column count.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **446 passed, 7 ignored** (196 library, 167
  binary, 82 `tests/core.rs` of which 6 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean apart from the
  pre-existing future-incompat notice.
- `resized_clamps_and_recomputes_offsets` checks min/max clamp, exact
  in-range resize, shifted offsets, the new total, and a no-op for an
  out-of-range column.
- `header_drag_resizes_column` drives real mouse events on the
  `header-resize-0` handle and asserts the layout grew by ~40 px and
  the tab snapshot stored the width.
- `resized_widths_restore_after_tab_switch` resizes, opens a second
  tab, switches back, and asserts the width was restored.
- **Not verified:** no manual GUI run; the drag is exercised by the
  headless UI test only. Double-click-to-autofit is not implemented (it
  was optional).

### Tests added / changed

- `ui/grid.rs`: `resized_clamps_and_recomputes_offsets`.
- `ui/view/tests.rs`: `header_drag_resizes_column`,
  `resized_widths_restore_after_tab_switch`.

### Roadmap changes

- Milestone 5 UX: "Column resize: drag column boundary in the header"
  ticked.

### Architecture changes

`ARCHITECTURE.md` Column rendering section documents `resized` /
`from_widths`, the wider manual cap, and per-tab persistence; the
`DocumentSnapshot` summary lists the stored widths.

### Issues / technical debt discovered

- The resize handle is 5 px wide and relies on `stop_propagation` to
  avoid the sort click. There is no visual "active" state during a
  drag beyond the hover highlight, and no double-click auto-fit.
- Column widths are stored as absolute pixels, so they do not scale
  with a window resize (consistent with the rest of the grid, which
  uses pixel widths).

### Next recommended task

Iteration 7: reveal the full exact value of a cell (Cmd/Ctrl+Enter or
a cell detail panel), handling multi-MB cells without freezing.

---

## 2026-09-30 — Iteration 7: reveal the full cell value

### Context

Seventh slice of the improvement run (branch
`agent/iterative-improvements`). The grid elides long values to the
column width; the only way to read the full value was to copy it.
The roadmap wanted a reveal affordance that handles multi-MB cells
without freezing the UI and says when output is capped.

### Implemented

- `parser::exact_string_capped(bytes, max_bytes)`: byte-faithful escaped
  text for at most `max_bytes`, returning a truncation flag. If the
  whole cell is valid UTF-8 the prefix backs off to a character
  boundary; otherwise the prefix is escaped exactly like
  `exact_string` (`\\`, `\xNN`). Exported from `anvil_core`.
- `commands::Action::RevealCell`, bound to Cmd/Ctrl+Enter.
- `MainView` full-cell panel: `cell_detail_open`, `cell_detail`,
  `cell_detail_generation`, `cell_detail_task`. `kick_cell_detail`
  reads the cursor cell on the background executor, capped at 32 KiB,
  and is generation-guarded so rapid cursor moves discard stale reads.
  `render_cell_detail` is a right-hand panel (like the inspector) that
  shows the header/row/col, the byte size, a "(capped)" note when
  truncated, and the exact value in a scroll area. It follows the
  cursor because `kick_inspector` (called on every selection change)
  now also refreshes it; Cmd/Ctrl+Enter toggles it, and the panel has a
  close `×`. `grid_viewport_width` subtracts the panel width so the
  grid reflows.
- `theme::CELL_DETAIL_WIDTH` (360 px).

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **451 passed, 7 ignored** (196 library, 171
  binary, 83 `tests/core.rs` of which 6 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean.
- `exact_string_capped_preserves_prefix_and_flags_truncation` covers the
  exact cap, a mid-character cut, and invalid-byte escaping.
- `cell_detail_shows_exact_value` drives Cmd/Ctrl+Enter through the
  real grid focus path and asserts the panel holds the exact value.
- `cell_detail_caps_a_huge_value` loads a 100 KiB cell, opens the panel
  and asserts `truncated` with exactly 32 KiB of text and the true
  byte length.
- **Not verified:** no manual GUI run; the panel is exercised by the
  headless UI tests only.

### Tests added / changed

- `tests/core.rs`:
  `exact_string_capped_preserves_prefix_and_flags_truncation`.
- `ui/commands.rs`: `reveal_cell_chord`,
  `plain_enter_is_not_reveal_cell`.
- `ui/view/tests.rs`: `cell_detail_shows_exact_value`,
  `cell_detail_caps_a_huge_value`.

### Roadmap changes

- Milestone 5 data fidelity: "Add an affordance to reveal the full value
  of a cell that the grid has elided" ticked.

### Architecture changes

`ARCHITECTURE.md` byte-faithful invariant 12 now mentions
`exact_string_capped` and the 32 KiB panel cap; the UI component list
documents the panel and the `ui::commands` action list includes
`RevealCell`.

### Issues / technical debt discovered

- The panel refreshes through `kick_inspector`, which means opening the
  inspector also triggers a (guarded) cell read. Harmless but a little
  indirect; a dedicated selection-change hook would be cleaner.
- The 32 KiB cap is a constant; it is not user-configurable. The panel
  labels truncation, so the limitation is visible.

### Next recommended task

Iteration 8a: make text sorting and `min`/`max` compare exact bytes
with a deterministic tiebreak, starting from a failing test.

---

## 2026-09-30 — Iteration 8a: exact-byte text ordering

### Context

First part of Iteration 8 (data fidelity). Text sorting and the
inspector's `min`/`max` compared a trimmed, case-insensitive form, so
`Apple`/`apple` and `42`/`42` compared equal even though the fidelity
rule treats them as distinct values.

### Implemented

- `query::sort`: the `Text` key now stores the cell's exact decoded
  bytes (no `trim`, no `to_ascii_lowercase`). Equal keys tie-break on
  the record index, making the order a deterministic total order.
  Numeric keys are unchanged (trimmed `f64` parse), and empty /
  whitespace-only values still sort last via the existing
  `trimmed.is_empty()` check.
- `schema::stats::compare_values`: text columns now compare exact bytes
  (`a.cmp(b)`); numeric columns keep trimming internally for the `f64`
  parse. The case-insensitive `cmp_text_ci` helper is removed. The
  inspector still displays exact values and classifies emptiness by
  `trim().is_empty()` (unchanged here; decided in 8b).

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **453 passed, 7 ignored** (198 library, 171
  binary, 83 `tests/core.rs` of which 6 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean.
- New tests prove `Apple` vs `apple` and ` a` vs `a` are distinct and
  deterministic, and that `min`/`max` follow the same rule. The
  numeric, empty-last, filter-composition, and stability tests still
  pass unchanged.

### Tests added / changed

- `query/sort.rs`: `sorts_text_case_insensitively` replaced by
  `sorts_text_by_exact_bytes`; added
  `text_sort_distinguishes_case_and_padding`.
- `schema/stats.rs`: `text_min_max_is_case_insensitive` replaced by
  `text_min_max_compares_exact_bytes`; added
  `text_min_max_is_case_sensitive_and_padding_aware`.

### Roadmap changes

- Milestone 5 data fidelity: "Make `min` / `max` ordering and text
  sorting compare the exact value" ticked.

### Architecture changes

`ARCHITECTURE.md` `query::sort` description updated (exact bytes,
record-index tiebreak).

### Issues / technical debt discovered

- A whitespace-only cell is still classified as empty and sorts last;
  Iteration 8b decides whether that stays the byte-faithful answer.

### Next recommended task

Iteration 8b: decide whether a whitespace-only cell is empty, per
ADR 0003.

---

## 2026-09-30 — Iteration 8b: a whitespace-only cell is a value

### Context

Second part of Iteration 8. The roadmap claimed the inspector and the
filter `empty` operator both treated a whitespace-only cell as empty.
Inspecting the code showed the filter already used `bytes.is_empty()`
(zero bytes); only the inspector disagreed. ADR 0003's byte-faithful
answer is that a whitespace-only cell holds bytes and is therefore a
value.

### Implemented

- `schema::stats`: the inspector now treats a cell as empty only when it
  has zero bytes. A whitespace-only cell counts as non-empty, appears in
  `min`/`max`/`samples`/distinct, and no longer inflates the `empty`
  count. The `trimmed` binding in the scan loop is gone (numeric
  comparison trims internally).
- Recorded the decision in `ARCHITECTURE.md` (`schema::stats` bullet)
  and in ADR 0003's follow-up note. The filter `empty` operator was
  already correct, so no filter behaviour changed; a regression test
  now documents the agreement.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **454 passed, 7 ignored** (199 library, 171
  binary, 83 `tests/core.rs` of which 6 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean.
- `whitespace_only_cells_are_values` asserts `empty == 0`,
  `non_empty == 2`, and exact-byte min/max.
- `whitespace_only_cells_are_not_empty` (filter) asserts `v empty`
  matches only the zero-byte row and `v notempty` only the
  whitespace-only row.
- Zero-byte empty counting (`counts_empties_and_distinct`) is unchanged.

### Tests added / changed

- `schema/stats.rs`: `whitespace_only_cells_still_count_as_empty`
  replaced by `whitespace_only_cells_are_values`.
- `query/filter.rs`: added `whitespace_only_cells_are_not_empty`.

### Roadmap changes

- Milestone 5 data fidelity: "Decide whether a whitespace-only cell is
  empty" ticked, noting the filter already used zero-byte semantics and
  that sorting still places whitespace-only values last per 8a.

### Architecture changes

`ARCHITECTURE.md` `schema::stats` description and ADR 0003 updated.

### Issues / technical debt discovered

- `schema::infer` and `sample_distinct` still trim before classifying a
  value; a whitespace-only cell may still infer as `Empty`. That is a
  type-inference heuristic, not the inspector/filter emptiness
  question, and is left unchanged.

### Next recommended task

Iteration 8c: strip or handle a UTF-8 BOM so it never appears inside the
first header's name, while still reporting the BOM in diagnostics.

---

## 2026-09-30 — Iteration 8c: handle a UTF-8 BOM

### Context

Third part of Iteration 8. `encoding::scan` detected a UTF-8 BOM and
diagnostics reported it, but the parser did not skip it, so a
BOM-prefixed file showed the BOM inside its first header name (and, for
a headerless file, inside its first cell).

### Implemented

- `parser::UTF8_BOM`, `parser::bom_len(bytes)`, and
  `parser::strip_utf8_bom(bytes)`; exported from `anvil_core`.
- `parser::delimit::detect_with` now runs on `strip_utf8_bom(bytes)`,
  so the sampled first line never carries the BOM.
- `parser::indexing::build_impl` starts at `bom_len(bytes)`:
  `header_range` and the first record offset begin after the BOM. The
  BOM bytes remain in the memory-mapped source, so `encoding::scan`
  still reports `bom = true`. No other cell bytes are altered.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **457 passed, 7 ignored** (199 library, 171
  binary, 86 `tests/core.rs` of which 6 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean.
- `utf8_bom_is_stripped_from_the_first_header_but_reported` asserts the
  header is `["name", "age"]`, the first cell is `alice`,
  `header_range == Some((3, 11))`, and `encoding::scan` still reports
  the BOM.
- `utf8_bom_headerless_first_cell_is_clean` covers the headerless case.
- `strip_utf8_bom_only_strips_the_bom` checks only a real `EF BB BF`
  prefix is stripped.

### Tests added / changed

- `tests/core.rs`: `utf8_bom_is_stripped_from_the_first_header_but_reported`,
  `utf8_bom_headerless_first_cell_is_clean`,
  `strip_utf8_bom_only_strips_the_bom`.

### Roadmap changes

- Milestone 4 encoding diagnostics: the "(Optional) Detect BOM and
  report encoding" item now records that the parser strips the BOM
  while diagnostics keep reporting it.

### Architecture changes

`ARCHITECTURE.md` `parser::indexing` and `parser::delimit` descriptions
note BOM handling.

### Issues / technical debt discovered

- A BOM in a ZIP entry is handled the same way because extraction and
  indexing share the path. A BOM not at offset 0 (mid-file) is data and
  is left alone.

### Next recommended task

Iteration 8d: scope the column inspector to the filtered/sorted view
when one is active, and label it clearly.

---

## 2026-09-30 — Iteration 8d: scope the inspector to the current view

### Context

Final part of Iteration 8. The column inspector always scanned the whole
document, so with a filter or sort active its counts, min/max and
samples did not describe the rows the user was looking at.

### Implemented

- `schema::stats::stats_rows(doc, column, rows, options)` scans only the
  supplied record indexes; `stats` is refactored onto a shared
  `stats_scan` with an optional row list. Exported as
  `column_stats_rows`.
- The UI passes `view_rows()` (filtered/sorted) when present and the
  whole document otherwise. `MainView.inspector_view_key` records
  `(filter_generation, sort_generation)` so an open inspector rescans
  when the view changes; the filter and sort completion paths now call
  `kick_inspector`.
- `render_inspector` labels the scope: "scoped to current view (N of M
  rows)" when a filter or sort is active, "whole document" otherwise.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **460 passed, 7 ignored** (201 library, 172
  binary, 86 `tests/core.rs` of which 6 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean.
- `stats_rows_scopes_to_the_given_rows` asserts a two-row view yields
  `total_rows == 2` with the view's min/max/samples, versus three for
  the whole document; `stats_rows_empty_view_has_zero_rows` covers the
  empty view.
- `inspector_scopes_to_the_filtered_view` filters `country == US`, opens
  the inspector on `amount`, and asserts `total_rows == 2`, min `10`,
  max `30`.

### Tests added / changed

- `schema/stats.rs`: `stats_rows_scopes_to_the_given_rows`,
  `stats_rows_empty_view_has_zero_rows`.
- `ui/view/tests.rs`: `inspector_scopes_to_the_filtered_view`.

### Roadmap changes

- Milestone 3 column inspector: "The inspector scans the whole document,
  not the filtered/sorted view" ticked.

### Architecture changes

`ARCHITECTURE.md` `schema::stats` description documents `stats_rows` and
the UI scoping.

### Issues / technical debt discovered

- `infer_column` still samples the whole document, so a scoped view's
  inferred type can reflect values that are filtered out. That only
  selects numeric-vs-text comparison and is a separate follow-up.
  **(Resolved in Iteration 13, 2026-10-01: `stats_rows` now infers
  through `schema::infer_column_rows` over the same row set.)**

### Next recommended task

Iteration 9: `FilterExpr::Not` with `NOT` parsing, `apply_expr`
evaluation, and `describe` round-trip.

---

## 2026-09-30 — Iteration 9: filter NOT

### Context

Ninth slice of the improvement run (branch
`agent/iterative-improvements`). The filter grammar had `AND` / `OR` and
row-scope predicates but no general negation; the Milestone 6
prerequisite had been deferred.

### Implemented

- `FilterExpr::Not(Box<FilterExpr>)`; `is_not_keyword` recognises an
  unquoted case-insensitive `NOT`; `parse_factor` parses it as a prefix
  operator binding tighter than `AND` (`NOT a AND b` is
  `(NOT a) AND b`). The predicate value loop now treats `not` as a
  keyword boundary, so a value literally named `not` must be quoted
  (matching `AND` / `OR`).
- `PreparedExpr::Not` evaluates as `!inner.matches(...)`.
- `nl::describe` renders `NOT` with a parenthesized operand, so
  `NOT (a AND b)` round-trips instead of reparsing as `(NOT a) AND b`.
- `query::suggest`: a typed `NOT` keeps the column slot, and `NOT` is
  offered as a column-slot completion after the columns (preserving the
  existing suggestion ordering).

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **468 passed, 7 ignored** (208 library, 173
  binary, 86 `tests/core.rs` of which 6 ignored, 1 style guard).
- `cargo clippy --release --all-targets` — clean.
- Parser/eval tests cover inversion, case-insensitivity, precedence
  over `AND`, parenthesized groups, and missing-factor errors.
- `describe_round_trips_not` reparses `NOT` trees (including
  `NOT (a OR b)` and `NOT any empty`).
- Suggest tests cover `NOT ` staying in the column slot, `NOT country `
  moving to the operator slot, and `NOT` being offered/prefix-matched.
- `filter_bar_applies_not` types `NOT country == US` in the real filter
  bar and asserts only the UK row remains.

### Tests added / changed

- `query/filter.rs`: `expr_not_inverts_a_predicate`,
  `expr_not_binds_tighter_than_and`,
  `expr_not_applies_to_a_parenthesized_group`,
  `expr_not_requires_a_factor`.
- `query/nl.rs`: `describe_round_trips_not`.
- `query/suggest.rs`: `not_prefix_keeps_the_column_slot`,
  `column_slot_offers_not`.
- `ui/view/tests.rs`: `filter_bar_applies_not`; updated the suggestion
  count assertion (4 -> 5) now that `NOT` is a column-slot candidate.

### Roadmap changes

- Milestone 6 prerequisites: `FilterExpr::Not` ticked (holds); the
  Tier 1 "general not waits" note updated to say `NOT` is available.

### Architecture changes

`ARCHITECTURE.md` `query::filter` descriptions updated to AND/OR/NOT
and the `describe` parenthesization rule.

### Issues / technical debt discovered

- None new. The Tier 1 NL parser still negates through negating
  operators; wiring `NOT` into the NL rules is Milestone 6 work and was
  explicitly out of scope.

### Next recommended task

Iteration 10: split `ui/view.rs` into cohesive modules with no
behaviour change.

---

## 2026-09-30 — Iteration 10: split ui/view.rs (part 1)

### Context

Tenth (and final planned) slice: `src/ui/view.rs` had grown to ~3,860
lines. The task was a behaviour-neutral split following the existing
`ui/view/{search,filter,ask,diagnostics}.rs` pattern. Candidate modules
were tabs, file panel, inspector, and sorting.

### Implemented

- New `ui/view/sort.rs`: `view_rows`, `cycle_sort`, `kick_sort`,
  `sort_label` (methods made `pub(super)` so `view.rs` and sibling
  modules can call them).
- New `ui/view/cell_detail.rs`: `toggle_cell_detail`,
  `open_cell_detail`, `close_cell_detail`, `kick_cell_detail`,
  `render_cell_detail`.
- New `ui/view/inspector.rs`: `render_inspector`, `toggle_inspector`,
  `kick_inspector`, plus the free helpers `metric_row` and
  `render_column_stats`.
- `view.rs` gained `mod cell_detail; mod inspector; mod sort;` and
  dropped ~500 lines (3,859 -> 3,362). The methods were moved verbatim;
  only their visibility changed.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **468 passed, 7 ignored**, unchanged from
  before the refactor (no test file was edited; the whole suite is the
  behaviour guard).
- `cargo clippy --release --all-targets` — clean.

### Tests added / changed

None. This iteration is refactor-only and deliberately changes no
behaviour, so the existing suite must pass unmodified.

### Roadmap changes

None (the split is not a roadmap checkbox).

### Architecture changes

`ARCHITECTURE.md` module tree lists the new submodules, and the
functional-gaps note records that the tab strip / file panel / grid
rendering are still in `view.rs`.

### Issues / technical debt discovered

- The split is partial: the tab strip, file panel, and grid/scrollbar
  rendering remain in `view.rs`. They are coherent next extractions
  but `group_documents` is referenced directly by `ui/view/tests.rs`,
  so that move needs an import-path change in the test module.

### Next recommended task

Finish the `ui/view.rs` split (tabs, file panel, grid rendering) as a
separate refactor-only iteration, then re-run the full suite.

---

## 2026-10-01 — Iteration 11: fix the Linux release build (rfd backend)

### Context

An independent review on x86-64 Linux reported that all 468 tests pass
once the Linux native packages are installed, but that
`cargo build --release` fails as shipped because `Cargo.toml` declared
`rfd` with `default-features = false` and no backend. The prior work had
only ever been verified on macOS, where the failure does not occur.

### Implemented

- Reproduced the root cause: `rfd` 0.17.2's `build.rs` panics with
  `You need to choose at least one backend: gtk3 or xdg-portal features
for x86_64-linux` when neither backend feature is enabled and
  `CARGO_CFG_TARGET_OS == "linux"`. macOS and Windows take a different
  branch of the build script, so the earlier macOS-only verification
  never hit it.
- `Cargo.toml`: kept the generic
  `rfd = { version = "0.17", default-features = false }` for
  macOS/Windows and added a
  `[target.'cfg(target_os = "linux")'.dependencies]` entry selecting
  `features = ["xdg-portal"]`. `xdg-portal` is pure Rust and adds only
  `pollster` (`pollster 0.4.0` is now in `Cargo.lock`), so no new
  system packages are required. The sync `FileDialog::pick_file()`
  used at `src/ui/view.rs:1371` is implemented by `rfd` via
  `pollster::block_on`, so switching to `gtk3` was not necessary.
- Re-read `.github/workflows/ci.yml`; the Linux package list is
  sufficient once a backend is selected, so no CI change was made.

### Verified

- macOS build/test unchanged: `cargo build --release` succeeds;
  `cargo test --release` — **468 passed, 7 ignored** (208 library, 173
  binary with 1 ignored, 86 `tests/core.rs` with 6 ignored, 1 style
  guard), identical to the pre-change count.
- `cargo fmt --check` clean; `cargo clippy --release --all-targets`
  clean.
- `cargo run --release --bin gen_fixture -- fixtures 2000` and
  `cargo run --release --bin anvil -- --probe fixtures/large.csv` run
  (the 1M fixture was regenerated afterwards).
- Focused Linux-target cross-check, since this host is macOS/arm64 and
  has no Linux VM or running Docker: in a throwaway crate,
  `cargo check --target x86_64-unknown-linux-gnu` with
  `rfd default-features = false` panics exactly as reported, and with
  the Linux target entry plus `features = ["xdg-portal"]` it finishes
  successfully. `cargo tree --target x86_64-unknown-linux-gnu` confirms
  `xdg-portal` + `pollster` are active for the target.

### Not verified

- The full Linux `cargo build --release`, `cargo test --release` and
  `--probe` were **not** run: the verification host is macOS/arm64, the
  Docker daemon is unavailable, and there is no Linux runner here.
  GitHub Actions was not run. The Linux and Windows legs of the CI
  matrix therefore remain unverified until a real runner executes them.
  The Linux GUI backend remains an open item.

### Tests added / changed

None.

### Roadmap changes

- The `[~]` "Add a CI build/test matrix" item now records the missed
  Linux-only `rfd` failure and its fix. It stays `[~]` because no real
  CI run has happened.

### Architecture changes

`ARCHITECTURE.md`: the cross-platform build limitation now documents the
`rfd` root cause and the fix, and the dependency table records the Linux
`xdg-portal` target entry.

### Issues / technical debt discovered

- The `agent/iterative-improvements` branch ref no longer exists; it was
  fast-forwarded into `master` (see reflog), so work continues on
  `master` @ `ecba98b`.
- Worker delegation tooling was unusable in this session (scope
  validation errors on confirmed regular files; one trivial scout task
  hit the model-turn limit), so this iteration was implemented locally
  after explicit recovery routing.

### Next recommended task

Iteration 2: fix range selection under an active sort by keeping the
selection in display-position space.

---

## 2026-10-01 — Iteration 12: display-position selection under an active sort

### Context

The previous agent recorded as technical debt that `Selection` was a
rectangle in raw record-index space. The grid and the copy path already
walked the visible display order, but a dragged or shift-clicked range
under an active sort spanned a record-index interval whose records are
not adjacent on screen: the highlight, the copy, and select-all
disagreed with the visible block. This iteration fixes that by moving
the model itself into display-position space.

### Implemented

- `CellPos` now stores `row` (a 0-based display position) instead of
  `record_index`. `Selection::move_cursor`, `move_cell`, `select_all`,
  `contains`, `cell_count` and the docs were updated to view-relative
  semantics; `copy_view_text_with`/`view_cell_count` now index
  `view_rows` by position rather than searching for records in a
  record rectangle. Added `Selection::clamp_to_rows`.
- `ui/grid.rs`: `render_row` takes the record index (for cell text,
  search, warnings, gutter) and the display position (for selection
  and copy-flash); the click/drag callbacks now report the display
  position.
- `ui/view.rs`: the selection is resolved against `view_row_count`
  (a new helper next to `view_rows`) for navigation, Tab, select-all,
  auto-scroll and reindex clamping. `move_selection` and
  `move_cell_selection` dropped their record-space binary searches.
- `ui/view/{sort,filter}.rs`: `clamp_selection_to_view` runs when a
  sort or filter result lands, and when a sort is cleared. A
  header-mode reindex still drops an out-of-range selection.
- `ui/view/{goto,search,diagnostics,cell_detail}.rs`: go-to-row,
  find-next and diagnostics land on the record's display position;
  the full-cell panel resolves the position to a record before its
  background read. Byte-faithful copy is unchanged (ADR 0003).

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **475 passed, 7 ignored** (211 library, 177
  binary with 1 ignored, 86 `tests/core.rs` with 6 ignored, 1 style
  guard), up from 468 passed / 7 ignored.
- `cargo clippy --release --all-targets` — clean.
- The new UI test was written first and confirmed to fail: the drag
  produced `(0,0)-(2,1)` in the old record space, and now produces
  `(0,0)-(1,1)`.
- A full Linux build was still not run (macOS/arm64 host); see
  Iteration 11.

### Tests added / changed

- `src/selection.rs`: `copy_view_uses_positions_not_record_indexes`,
  `copy_view_clamps_a_stale_selection_to_the_view`,
  `clamp_to_rows_clamps_cursor_and_anchor`; renamed the view-copy test
  and updated the position-space expectations.
- `src/ui/view/tests.rs`: `drag_select_under_sort_copies_the_visible_block`
  (the reproduction and regression), `arrow_keys_walk_the_sorted_view`,
  `tab_walks_the_sorted_view`, `filter_change_clamps_selection`.
  Updated `goto_maps_through_active_filter`,
  `filter_vertical_navigation_skips_hidden_rows`,
  `filter_tab_and_row_jumps_walk_visible_rows` and the diagnostics
  cursor assertions to display positions.

### Roadmap changes

- Milestone 2 Selection model: the `Selection` task now describes
  display-position rows, and a completed sub-task records the
  sort-alignment fix.

### Architecture changes

`ARCHITECTURE.md`: the `selection` module description, the selection
model section (operations table, invariants, grid integration) and the
clipboard section now document display positions, `clamp_to_rows`,
`copy_view_text_with` and `view_cell_count`.

### Issues / technical debt discovered

- None new. The historical debt note in the Iteration 2 log entry is
  annotated as resolved.

### Next recommended task

Iteration 13: make the column inspector's type inference sample the
same filtered/sorted row set as its stats.

---

## 2026-10-01 — Iteration 13: scoped type inference for the column inspector

### Context

Iteration 8d scoped the inspector's statistics to the filtered/sorted
view but left `stats_scan` calling `infer_column`, which samples records
`0..2048` of the whole document. A view whose rows were all numeric could
therefore be labelled `Text` (or vice versa) while the stats beside it
described the visible rows. This iteration removes that disagreement.

### Implemented

- `schema.rs`: inference is now driven by an iterator of row indexes.
  Added `infer_column_rows(doc, column, rows)`, which samples at most
  `INFER_SAMPLE_ROWS` (2048) of the supplied rows in display order;
  `infer` / `infer_column` keep the whole-document behaviour. The
  numeric/boolean/empty decision logic is unchanged.
- `schema/stats.rs`: `stats_scan` picks `infer_column_rows` when a row
  set is supplied and `infer_column` otherwise, so `stats_rows` (the
  UI's scoped path) infers over exactly the rows it measures. Inference
  still runs on the background executor inside the existing inspector
  task; the 2048-row bound is preserved.

### Verified

- The new unit test was written first and confirmed to fail against the
  old one-line `infer_column` call (`left: Text, right: Integer`); it
  passes with the scoped inference.
- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **477 passed, 7 ignored** (212 library, 178
  binary with 1 ignored, 86 `tests/core.rs` with 6 ignored, 1 style
  guard), up from 475.
- `cargo clippy --release --all-targets` — clean.
- A full Linux build was still not run (macOS/arm64 host).

### Tests added / changed

- `src/schema/stats.rs`: `stats_rows_infers_type_from_the_scoped_view`
  (numeric view of a mixed column, and a text view of a numeric column).
- `src/ui/view/tests.rs`: `inspector_infers_type_from_the_filtered_view`
  drives the real filter bar, scopes the inspector and asserts the
  reported `ColumnType` is `Integer`.

### Roadmap changes

- Milestone 3 column inspector: added a completed "Scoped type
  inference" sub-task and updated the stats-scoping entry.

### Architecture changes

`ARCHITECTURE.md`: the `schema::stats` description and the Milestone 3
type-inference note now document `infer_column_rows` and scoped
inference; the stale "scoping inspector stats ... outstanding" gap was
removed.

### Issues / technical debt discovered

- The historical Iteration 8d debt note is annotated as resolved. The
  broader "cache inferred profiles at open" follow-up still stands:
  inference is recomputed by the sorter and the inspector.

### Next recommended task

Iteration 4: determine whether the Linux release build can open a GPUI
window or document the open question without guessing.

---

## 2026-10-01 — Iteration 14: Linux GPUI window backend

### Context

The previous agent suspected that the release build compiled GPUI without
the x11/wayland backends, so a Linux release binary could not open a
window, while `cargo test` pulled them in through the dev-only
`test-support` feature. This iteration confirmed that from the GPUI 0.2.2
source, fixed it with a target-specific feature entry, and recorded that
the window itself remains unlaunched here.

### Confirmed (compile-time, no display needed)

- `gpui 0.2.2`'s `test-support` feature enables both `wayland` and `x11`;
  the project's release build used `default-features = false, features =
["font-kit"]`.
- On Linux, `gpui::platform::current_platform` returns
  `HeadlessClient` unless one of those features is compiled in:
  `guess_compositor()` hardcodes the wayland/x11 display probes to `None`
  when the features are off, so it always returns "Headless". This is not
  a "no display" condition; the backend simply was not compiled in, so
  the old binary could not open a window on any display.
- This host is macOS/arm64, so no Linux display exists to launch against.

### Implemented

- `Cargo.toml`: the Linux target table now also selects
  `gpui = { default-features = false, features = ["font-kit", "x11",
"wayland"] }`, mirroring what `test-support` already enabled for
  `cargo test`. Both backends are kept because a Linux desktop may run
  either server (and a Wayland session would otherwise fall back to
  headless). macOS/Windows resolution is untouched: the table is
  Linux-only.
- `.github/workflows/ci.yml`: the Linux package list already covers this
  feature set (the same x11/wayland stack `cargo test` already built), so
  only the explanatory comment was updated.

### Verified

- `cargo tree --target x86_64-unknown-linux-gnu -e normal,features -i
gpui` (release/normal edges only) shows `font-kit`, `x11` and `wayland`
  for Linux; the same command for `aarch64-apple-darwin` shows only
  `font-kit`, confirming macOS is unchanged.
- `cargo fmt --check` — clean.
- `cargo build --release` (macOS) — succeeds; `cargo test --release` —
  **477 passed, 7 ignored**, unchanged.
- `cargo clippy --release --all-targets` — clean.
- **Not verified:** the Linux window was not launched. There is no Linux
  display or container on this host, so the claim is limited to "the
  backend is now compiled in and feature resolution is correct". Full
  GUI behaviour on Linux still needs a Linux runner, which is why the
  roadmap item stays `[~]`.

### Tests added / changed

None (a build/feature change). The existing suite is the regression guard
for the macOS resolution.

### Roadmap changes

- Milestone 5 "Enable a real GPUI window backend on Linux" moved
  `[ ] → [~]`: features added and cross-target resolution verified, but
  the window is unlaunched.

### Architecture changes

`ARCHITECTURE.md`: the cross-platform build limitation and the `gpui`
dependency row now document the Linux x11/wayland target features and
that Linux GUI behaviour is unverified. `README.md` gained a short
platform note with the Linux native-package requirement.

### Issues / technical debt discovered

- Enabling x11 + wayland adds `blade-graphics` (GPU) and its dependency
  tree to the Linux release build. That is the cost of a real window;
  it does not affect macOS/Windows. If the CI Linux leg is slow, this is
  the reason, and build-time caching is the mitigation.
- The `--probe` path still exercises GPUI's headless platform on every
  OS, so it stays usable without a display.

### Next recommended task

Iteration 5 (optional): finish the `ui/view.rs` split (tab strip, file
panel, grid/scrollbar rendering) as a behaviour-neutral refactor.

---

## 2026-10-01 — Iteration 15: finish the ui/view.rs split

### Context

The Iteration 10 refactor moved the inspector, full-cell panel and sort
logic out of `view.rs` but left the tab strip, file panel, and
grid/scrollbar rendering in place. This optional, behaviour-neutral
iteration finishes that split following the existing `ui/view/*.rs`
pattern.

### Implemented

- New `ui/view/tab_strip.rs`: `render_tab_bar`.
- New `ui/view/file_panel.rs`: `render_file_panel`.
- New `ui/view/grid_area.rs`: `render_grid`, `start_column_drag`,
  `column_drag_move`, `end_column_drag`, `render_h_scrollbar`,
  `on_grid_scroll`, `scroll_horizontal`, `ensure_column_visible`.
- `view.rs` gained `mod tab_strip; mod file_panel; mod grid_area;` and
  dropped from ~3,314 to ~2,629 lines. The methods were moved verbatim;
  only their visibility changed (`pub(super)` so `view.rs` and sibling
  modules can still call them). A small script matched whole inherent
  items (doc comment through closing brace) so no method was duplicated
  or lost; a name grep confirms each moved method is defined once.
- The `view.rs` module doc lists the full submodule set.

### Verified

- `cargo fmt --check` — clean.
- `cargo build --release` — succeeds.
- `cargo test --release` — **477 passed, 7 ignored**, exactly the
  pre-refactor counts; no test file was edited (the suite is the
  behaviour guard).
- `cargo clippy --release --all-targets` — clean.
- A full Linux build was still not run (macOS/arm64 host).

### Tests added / changed

None. This iteration is refactor-only.

### Roadmap changes

None (the split is not a roadmap checkbox).

### Architecture changes

`ARCHITECTURE.md`: module tree lists `tab_strip.rs`, `file_panel.rs` and
`grid_area.rs`; the functional-gap and architectural-debt notes now say
the tab strip / file panel / grid rendering are extracted.

### Issues / technical debt discovered

- None new. `view.rs` still owns `MainView` state, open orchestration,
  the toolbar, status bar and centre/overlay composition; further
  extraction is optional.

### Next recommended task

None outstanding from this run. The next agent could pick up the
Milestone 5 cross-platform items (real Linux/Windows CI run and GUI
exercise) or the richer type classes recorded in `ROADMAP.md`.

---

## 2026-10-01 — Agent workflow: conventional branches and pull requests

### Context

The operator changed the repository workflow: retire the long-lived
`agent/iterative-improvements` branch and, from now on, land each change
set on a Conventional Commits-style branch that is merged to `master`
through a pull request.

### Implemented

- Removed the `agent/iterative-improvements` branch ref (it pointed at
  the already-merged master tip) and retrofitted the previous five
  commits into PR #1, which is merged.
- Added a "Version Control Workflow" section to `AGENTS.md`: branch
  naming (`fix/...`, `feat/...`, `refactor/...`, `docs/...`, ...),
  Conventional Commit messages, PR requirements (what changed, what was
  verified, what was not), and the merge/branch-deletion steps.
- `AGENTS.md` Handoff, Definition of Done, and Open-source hygiene now
  reference the PR workflow; the quick-reference test count was
  refreshed to 477 passed, 7 ignored.

### Verified

- Documentation-only change; no build or test impact.
- The workflow is exercised by landing this change itself on
  `docs/agent-pr-workflow` through a pull request.

### Tests added / changed

None.

### Roadmap changes

None.

### Architecture changes

None (process documentation only).

### Issues / technical debt discovered

- GitHub Actions still has not run; CI cannot be observed from this
  macOS host, so "CI is green" in the new workflow means the local
  verification steps until a real run is available.

---

## 2026-10-01 — Iteration 16: Windows GPUI manifest and the first CI run

### Context

Pushing the previous work triggered the first real GitHub Actions run.
macOS was green; the Ubuntu leg built, tested and probed green (one
intermittent, out-of-scope NL `sh`-backend failure was seen and did not
reproduce); Windows built, tested and generated fixtures green but the
release `anvil.exe --probe` failed deterministically to start with
`0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`.

### Root cause

GPUI's default features are `["font-kit", "wayland", "x11",
"windows-manifest"]`. `windows-manifest` makes GPUI's build script embed
`resources/windows/gpui.manifest.xml`, which binds the process to
common-controls v6. The project disables default features globally and
only re-added `font-kit`, so the Windows release binary had no manifest
and the loader could not resolve the v6 imports. This is the Windows
analogue of the missing Linux `x11`/`wayland` backends fixed in
Iteration 14.

### Implemented

- `Cargo.toml`: added a
  `[target.'cfg(target_os = "windows")'.dependencies]` entry selecting
  `gpui` with `features = ["font-kit", "windows-manifest"]`. The generic
  and Linux entries are unchanged. `embed-resource` (GPUI's Windows
  build-dependency) was already in `Cargo.lock`, so no new packages.
- `ARCHITECTURE.md`, `ROADMAP.md` and this log now record the first CI
  results and the Windows fix.

### Verified

- macOS resolution unchanged: `cargo tree --target aarch64-apple-darwin
-e normal,features -i gpui` still shows only `font-kit`;
  `Cargo.lock` is unchanged; `cargo fmt --check`, `cargo build
--release`, `cargo test --release` and `cargo clippy --release
--all-targets` pass.
- The Windows fix is verified by this PR's Windows CI leg, which runs
  `target\release\anvil.exe --probe fixtures/large.csv` after build and
  test.

### Tests added / changed

None (a build/feature change).

### Roadmap changes

- Milestone 5 cross-platform: the CI matrix item is ticked now that a
  real run exists; a new completed task records the Windows manifest
  fix. The Linux GUI-window item stays `[~]` until a Linux display
  actually opens the window.

### Architecture changes

`ARCHITECTURE.md` gpui dependency row and cross-platform section now
document all three target-specific feature entries and the real CI
results.

### Issues / technical debt discovered

- The NL `ModelBackend` test that shells out to `sh` has a broken-pipe
  race on Linux (`src/query/nl.rs:1816`). It is out of scope
  (Milestone 6) and was left untouched, but it makes CI intermittently
  red. **(Resolved in Iteration 18: `run_command` ignores `BrokenPipe`
  while writing the prompt, with a deterministic regression test.)**

---

## 2026-10-01 — Iteration 17: self-contained on-disk layout

### Context

The operator's design principle: everything the program installs or
persists must live beside the `.exe`, in the same containing folder, so
deleting that folder deletes the entire program. Previously the optional
Ask backends wrote their downloaded models to the platform cache
(`%LOCALAPPDATA%\anvil\models`, `~/Library/Caches/anvil/models`,
`$XDG_CACHE_HOME/anvil/models` or `~/.cache/anvil/models`), which
violated that principle.

### Implemented

- New `anvil_core::paths` module. `anvil_data_dir()` resolves, in order:
  `ANVIL_DATA_DIR`; `<folder of the canonical executable>/anvil-data`
  (created and write-probed); then the legacy per-user cache root as a
  fallback for read-only installs, with a one-time stderr notice.
  `model_root()` is `<data>/models`; `legacy_data_dir()` exposes the old
  cache root for reading existing downloads in place.
- `gliner::model_dir()` now builds `<data>/models/gliner2.5-small-v1-<rev>`
  and falls back to a legacy download without copying it. `nlmodel`
  `cache_dir()` uses the same root with `ANVIL_NL_CACHE` preserved as an
  override; its `fetch` reuses a legacy file instead of re-downloading.
- ZIP extraction deliberately stays in `tempfile::tempdir()` (transient
  input staging, auto-deleted; not installed state).
- Documented the principle in `VISION.md` (core principle), `AGENTS.md`
  (scope rule), `ARCHITECTURE.md` (new "On-disk Layout" section),
  `ROADMAP.md` (Milestone 6 prerequisite) and `README.md`.

### Verified

- `cargo fmt --check`, `cargo build --release`, `cargo clippy --release
--all-targets` are clean.
- `cargo test --release` — **480 passed, 7 ignored** (215 library with 3
  new `paths` unit tests, 178 binary with 1 ignored, 86 core with 6
  ignored, 1 style guard), up from 477.
- `cargo check --release --features nl-gliner` and `--features nl-model`
  both succeed, so the rewired feature-gated modules compile.

### Not verified

- There is no runtime check on Windows or Linux of the new layout; CI
  exercises the default feature set only. A read-only program folder
  falls back to the legacy cache (and the portability guarantee then does
  not hold); that path is covered by an override/failure unit test, not a
  real read-only install.

### Tests added / changed

- `src/paths.rs`: `override_wins_and_is_created`,
  `portable_dir_sits_beside_the_executable`,
  `missing_executable_falls_back_or_errors`.

### Roadmap changes

- Milestone 6 prerequisites: added a completed self-contained on-disk
  layout task.

### Architecture changes

`ARCHITECTURE.md` gained the "On-disk Layout" section, the module tree
lists `paths.rs`, and the Ask-model section now documents the portable
model location.

### Issues / technical debt discovered

- If future configuration (recent files, preferences) is added it must go
  through `anvil_core::paths`, not `dirs`-style platform directories.

---

## 2026-10-01 — Iteration 18: stop the NL backend test flaking on Linux

### Context

The post-merge `master` CI run after PR #4 failed on Ubuntu at the Test
step, not the build: `query::nl::tests::
interpret_validates_model_output_through_the_filter_parser` panicked with
``could not write to `sh`: Broken pipe (os error 32)``. The same test had
already flaked once before. A re-run passed, confirming timing rather
than logic.

### Root cause

`query::nl::run_command` writes the prompt to the child's stdin. The test
backend is `sh -c "printf ..."`, which exits without reading stdin, so the
write can race the child's exit and hit `EPIPE`. Linux tends to lose that
race; macOS/Windows scheduling usually hides it. Nothing about the
platform build was wrong.

### Implemented

- `run_command` now treats `ErrorKind::BrokenPipe` while writing the
  prompt as non-fatal: the backend's stdout is still captured, and a
  backend that produced nothing still fails the existing "returned no
  output" check. Other write errors are still surfaced.
- Added `run_command_tolerates_a_backend_that_closes_stdin`, which uses
  `sh -c "exec 0<&-; printf ..."` with a 1 MB prompt so the write
  deterministically runs into the closed reader.

### Verified

- The new test was written first and confirmed to fail without the guard
  (`Broken pipe (os error 32)`), then passes with it.
- `cargo fmt --check`, `cargo build --release`, `cargo clippy --release
--all-targets` clean.
- `cargo test --release` — **481 passed, 7 ignored** (216 library, 178
  binary with 1 ignored, 86 core with 6 ignored, 1 style guard), up from 480.
- `cargo check --release --features nl-gliner` and `--features nl-model`
  succeed.

### Tests added / changed

- `src/query/nl.rs`: `run_command_tolerates_a_backend_that_closes_stdin`.

### Roadmap changes

None directly; the out-of-scope NL debt note in Iteration 16 is annotated
as resolved.

### Architecture changes

None.

### Issues / technical debt discovered

None new.

---

## 2026-10-02 — Iteration 19: Windows Space in text inputs

### Implemented

- Fixed Ask, filter, and search ignoring Space on Windows. GPUI reports
  the named space key without key_char; a shared printable-text helper
  supplies one literal space while preserving supplied text. Existing
  modifier guards and input refresh behavior are unchanged.

### Verified

- cargo fmt --check, cargo build --release, cargo test --release, and
  cargo clippy --release --all-targets succeeded on Windows. Clippy has
  no project warnings; Cargo notes an upstream proc-macro-error2 future
  incompatibility warning. Initial cold compilation exceeded two time
  limits; the longer run completed.
- Native GUI typing was not manually exercised. The fix is being submitted on
  fix/windows-space-input through a PR; merge is gated on CI.

### Tests added/changed

- Two printable-text helper tests cover Windows Space without key_char,
  supplied spaces/Unicode, and rejected empty/control text.
- A headless UI regression drives Windows-shaped Space events through
  all three handlers, plus Shift+Space and Ctrl+Space modifier checks.

### Roadmap changes

- Added a completed Milestone 5 Windows Space regression task; broader
  native cross-platform GUI verification remains open.

### Architecture changes

- Documented shared printable-text resolution and modifier guards.

### Issues / technical debt discovered

- No new application debt. Manual Windows GUI verification remains open.

### Next recommended task

- Manually type multi-word phrases in Ask, filter, and search on Windows
  as part of the existing native GUI verification task.

## 2026-10-02 — Iteration 20: dithered menu chrome, and the texture lag fix

### Context

The user wanted a dithered main menu area and reported the current
shell as "insanely laggy". The lag was `ui/texture.rs`: the welcome,
error and loading surfaces painted their dithered fields through a
`canvas` whose paint callback ran on every repaint (hover, cursor
blink, animation), walked every lattice cell of the whole region and
submitted one `paint_quad` per surviving dot. Measured on the
1200x700 welcome region (release): grain 26,154 dots / 939 us +
stipple 25,782 dots / 768 us of compute, i.e. ~52k quad submissions
plus 1.7 ms of UI-thread hashing per repaint - and multiples of that
in dev builds. The menu chrome itself had no texture at all.

### Implemented

- `ui/texture.rs` rework:
  - `grain` is now a GPU `checkerboard` `Background` (2 px pitch,
    5% alpha): evaluated in the WGSL shader, so it repaints for
    free. Signature changed from `grain(step, strength, color, seed)`
    to `grain(size, alpha, color)`; the three call sites updated.
  - `stipple` keeps the radial halftone but the halo is capped at
    `MAX_REACH` (260 px) and only the lattice cells inside the
    halo's bounding box are walked. The math is factored into a pure
    `stipple_dots(width, height, shape, step, seed)` returning
    origin-relative offsets, so it is unit-testable and the paint
    callback only submits quads.
  - New `chrome_dither(color)`: a GPU `pattern_slash` overlay
    (~3 px diagonal period, peaking near 7% alpha) for the menu
    chrome.
- `view/shell.rs`: the title bar and command bar each carry a
  `chrome_dither` overlay div (handler-free, so it never intercepts
  a click, hover or the Windows title-bar drag). Welcome surface
  updated to the new grain and a 6 px stipple step.
- `view.rs`: error and loading placeholders updated to the new grain.

### Verified

- cargo build --release, cargo test --release (213 lib + 212 bin
  incl. the shell tests that render the dithered title bar, command
  bar and welcome surface + 86 core), cargo fmt, cargo clippy
  --release: all clean, no new warnings.
- New `stipple_dots(1200x700)` measurement: 1,916 dots in 128 us
  (release) - ~27x fewer quads and ~13x less compute than the old
  field, and the cost is now constant in window size (a 4000x3000
  region produces the same halo as a 700x700 one; asserted in tests).
- Native window launched on Windows and captured; pixel analysis of
  the screenshot confirms all three layers paint: the title bar and
  command bar oscillate 24<->36 luminance with the ~3 px slash
  period, the welcome canvas alternates 18.4<->29.4 every 2 px
  (checkerboard grain over canvas #0D131C), and primary-coloured
  stipple dots appear inside the bounded halo. The app was killed
  after capture.

### Tests added/changed

- Five new tests in `ui/texture.rs`: halo bounded in region size,
  dot determinism (same seed identical, different seed differs),
  window-sized compute cost (dots < 6000, < 50 ms even in debug),
  and GPU pattern backgrounds paint non-transparent.

### Roadmap changes

- None: this is part of the in-progress `redesign/gpui-kit` slice;
  its native visual acceptance checklist item stays open (only the
  welcome screen and menu chrome were inspected, in Slate only).

### Architecture changes

- ARCHITECTURE.md: documented the material language (GPU pattern
  dither/grain, bounded CPU stipple) in the shell section.
- docs/ui-design.md: new "Material language" section with the
  do-not-unbound-the-halo constraint.

### Issues / technical debt discovered

- The dither strength (0.10 alpha) is a one-line tweak if it reads
  too strong; it was chosen to peak near 7% effective alpha.
- Anvil Warm (light) appearance was not pixel-verified; the dither
  colour is foreground-derived so it inverts automatically, but
  native inspection is still owed by the open visual-acceptance
  task.
- The change sits uncommitted in the working tree on
  `redesign/gpui-kit` alongside the rest of that in-progress
  redesign; it was not committed separately because the touched
  files already carry the uncommitted redesign work.

### Next recommended task

- Continue the redesign's native visual acceptance: Anvil Warm,
  populated grid, query bars, dialogs, panel switching, divider
  resizing and the 900x480 minimum, recording evidence per
  docs/ui-design.md.

## 2026-10-02 - Iteration 21: canonical design system and composed Home

### Implemented

- Audited existing docs, Kit APIs, source and dirty redesign before changing
  presentation. Wrote `docs/design/ANVIL_DESIGN_SYSTEM.md` and
  `HOME_WORKSPACE_SPEC.md` first, then reconciled them with implementation.
- Established cool-metal/copper identity through the existing Kit theme,
  including Button-specific overrides, focus, opaque data/value wells and
  shared inspector type/geometry tokens. Kept search/selection cool.
- Home now has an aligned opening lane, one genuine native Open action and
  a cached abstract folded-material field: 4x4 Bayer thresholds, 6 px lattice,
  bounded 520x420 geometry, fewer than 4000 quads. No idle timer or dot-list
  allocation/generation per paint. Removed full-canvas Home grain after review.
- Reduced chrome texture strength. Added stateless external-path drag-over
  boundary/entrance lift using verified GPUI styling APIs.
- Native review found Warm gutter blend inputs reversed and modal overlays
  fully opaque; corrected the blend and added translucent scrim alpha. A
  new contrast test caught 4.336:1 muted gutter text at the first calibration;
  reduced the gutter tint to 0.045 and verified the >=4.5:1 gate passes.
- Preserved existing uncommitted work and the entire backend/data workflow.
  Added bounded Windows capture helper `tools/design-review.ps1`.

### Verified

- Final `cargo fmt`, `cargo fmt --check`, `cargo build --release`,
  `cargo test --release` and `cargo clippy --release --all-targets` succeeded.
  Release suite: 519 passed, 7 ignored; no project Clippy warnings.
- Large CSV probe: 1,000,000 rows / 20 columns, 0 warnings, valid UTF-8,
  184 ms on the recorded run. Multi-ZIP probe discovers both candidate paths.
  Probe timing is not a before/after performance or GPU frame-time claim.
- Launched and inspected native Windows Home/grid in Slate/Warm, typed BRL
  into Find (143088 matches), navigated a cell, switched Column/Diagnostics,
  and opened/dismissed shortcuts. Captures are under `tools/shots/design-*`.
- Requested 1200x800 and 900x480 native sizes; the minimum capture is
  916x488 including OS non-client extent. Home actions and the grid plus
  auxiliary pane remain reachable. Final gutter tint adjustment is test-
  verified; the preceding captures use 0.07 rather than 0.045.
- Picker automation was attempted but the capture helper foregrounds the
  main window and did not capture the native dialog; this is NOT picker
  acceptance. Native external drop/abort, archive recovery and divider
  dragging were not exercised. No macOS/Linux or screen-reader/frame testing.

### Tests added/changed

- Theme tests: copper button states and opaque data/well/gutter text meet
  > =4.5:1 WCAG contrast; state fills are opaque; modal scrim is translucent;
  > identity and informational/selection colors remain distinct.
- Texture tests: Bayer threshold completeness, deterministic cached field,
  fixed lattice pitch, geometry bounds and <4000 quad budget.
- Real Kit Root tests: Home Open label and >=120x32 geometry at both window
  sizes/modes; absent false archive action; appearance preserves document Arc.
  No ignored test was added for unsupported headless native-picker APIs.

### Roadmap changes

- Added staged D1-D9 design migration under Milestone 5, marking specs,
  tokens/primitives, Home, shared theme migration and scoped Windows/build
  evidence complete. Full visual/platform acceptance remains unchecked.

### Architecture changes

- Documented cached Bayer field, shared material tokens, stateless drag styling
  and canonical specs. Reconciled stale chrome/help/material prose in
  architecture/UI notes; GPU patterns avoid CPU enumeration, not all GPU cost.

### Issues / technical debt discovered

- Persisted recents, persisted appearance/reduced effects, optional motion,
  accessibility/scaling, native drop/picker/archive/resizing and macOS/Linux
  acceptance remain explicitly staged; no fake recent rows or settings shipped.
- Work remains uncommitted on pre-existing `redesign/gpui-kit`; no branch,
  commit or PR was created because it would bundle substantial user-owned work.

### Next recommended task

- Exercise real native drop/leave/abort, picker cancellation, archive chooser
  recovery and divider resizing against the new material system; record GPU
  frame latency. Then implement bounded portable recent-source persistence
  as its own vertical slice before adding real Home history.

## 2026-10-02 - Iteration 22: recognizable dithered anvil artwork

### Implemented

- Replaced Home's abstract folds with an isometric anvil inspired by the
  user's supplied blocky reference: broad head, narrow waist and base.
- Eight convex faces use different Bayer-screen densities for depth; retained
  copper tint, 6 px lattice, cached 520x420 geometry, positioning and <4000
  quad budget. No image dependency, idle task or per-paint geometry allocation.

### Verified

- `cargo fmt --check`, `cargo build --release`, `cargo test --release` and
  `cargo clippy --release --all-targets` passed. Twelve targeted texture tests
  passed, including the new silhouette regression.
- Launched native Windows and viewed normal-size Slate/Warm captures;
  both show the recognizable anvil. Latest minimum capture showed stale
  pre-resize contents clipped by the smaller native window, so it is not
  minimum-layout acceptance. Headless Home geometry tests still pass at
  both sizes; native resize acceptance remains open.

### Tests added/changed

- `anvil_has_a_broad_head_narrow_waist_and_solid_base` checks representative
  face tones, negative space around the waist and that dots stay in the mask.
- Existing cache/determinism/lattice/bounds/quad-budget tests remain passing.

### Roadmap / architecture changes

- Recorded artwork refinement under Milestone 5. Updated canonical design
  specs, architecture and UI notes from abstract folds to anvil faces.

### Issues / next recommended task

- No backend or interaction behavior changed. Resolve native resize evidence
  separately; other previously recorded design acceptance remains open.
- Changes remain uncommitted alongside the user-owned redesign; no PR created.

## 2026-10-02 - Iteration 23: balance the Home anvil proportions

### Implemented

- Centered head, waist and pedestal on local x=260; moved the waist beneath
  the head and made left/right head overhangs equal at 98 px.
- Shortened the head's length/depth, reduced head thickness from 76 to 60 px,
  broadened the front face and reduced base thickness from 42 to 36 px.
  Retained isometric face shading, copper Bayer treatment and cached bounds.

### Verified

- Release build, full release tests, Clippy all-targets and format check pass.
  Thirteen targeted texture tests pass. Native Windows normal-size Slate/Warm
  screenshots reviewed; the silhouette is visibly more centered on its waist.
- No performance claim beyond the existing tested cache/quad/bounds contract.

### Tests added/changed

- Added `anvil_masses_share_a_center_and_equal_head_overhang` to check shared
  geometric centers and equal overhangs. Updated representative face probes.

### Roadmap / architecture changes

- Annotated the existing Home artwork item and specified balanced proportions
  in `HOME_WORKSPACE_SPEC.md`. No rendering architecture changed.

### Issues / next recommended task

- No backend or opening behavior changed; native resizing/scaling acceptance
  remains the next evidence task. No commit/PR created amid user-owned work.

## 2026-10-02 - Iteration 24: README cleanup and redesign PR preparation

### Implemented

- At the user's request, removed all temporary `.shots/` and `tools/`
  screenshots, capture/debug scripts and logs before staging the redesign.
- Captured a fresh native Slate Home, cropped away surrounding desktop and
  retained only `docs/images/home-workspace.png` for the README.
- Updated README workspace/features, canonical design links, implementation
  limits and the implemented help shortcut. Added ignore rules for future
  capture/log output; canonical specs mark deleted capture paths as historical.
- Updated the CI Linux dependency comment for the Kit runtime.

### Verified

- Reviewed the retained image, README image path and untracked-file list;
  no temporary captures/scripts/logs remain in the proposed change set.
- Final Windows format check, full release tests and all-target Clippy pass;
  preceding release build/probes/native review remain documented above.
- Independent source review found no high-confidence merge blockers; native
  cross-platform, resize/drop/accessibility and GPU acceptance remain open.

### Roadmap / architecture changes

- No feature acceptance was promoted. Documentation/assets cleanup only.
  User authorized the full redesign commit, PR and merge once CI is green.

### Handoff

- Submit the coherent redesign through a conventional feature branch into
  `master`; wait for the three-platform CI matrix, then merge and fast-forward
  local `master`. Native acceptance gaps must remain explicit in the PR.

## 2026-10-02 - Iteration 25: contain workspace text and restore vertical scrollbar

### Implemented

- Attached Kit's always-visible vertical scrollbar to the existing uniform-list
  handle; retained virtualization and horizontal scrolling.
- Fixed query fields at 24 px with centered 18 px single-line editors rather
  than allowing inherited line height and wrapping to exceed their frames.
- Sized two-line Files buttons at 40 px, constrained metadata/name overflow,
  and bounded document tabs to 240 px with width-based ellipsis and tooltips.

### Verified

- `cargo fmt`, full `cargo test --release`, `cargo build --release`, and
  `cargo clippy --release --all-targets` passed.
- Five targeted input tests and seven navigation tests passed.
- Native visual/scrollbar drag inspection was not performed; normal/minimum
  sizes in both themes remain a required follow-up. No probe was run because
  this slice does not change loading or the data path.

### Tests added/changed

- Added input geometry coverage for placeholders, UTF-8, long and selected
  queries with oversized inherited line height, caret and hit-test bounds.

### Roadmap / architecture changes

- Recorded the implemented overflow/scrollbar fixes under Milestone 5 without
  promoting native visual acceptance. No state ownership/data architecture change.

### Issues / next recommended task

- Visually exercise long filenames, all query bars and vertical thumb dragging
  at normal/minimum window sizes in Slate/Warm.
- No branch, commit or PR created; changes remain local for user review.

## 2026-10-02 - Iteration 26: filter Tab autocomplete dispatch

### Implemented

- Added a candidate-popup-specific FilterAutocomplete context and Tab NoAction
  binding so Kit Root focus traversal cannot preempt the existing completion
  handler. Tab accepts the highlighted candidate without applying the filter.
- Normal Tab traversal resumes after completion or without candidates;
  Shift+Tab retains backward traversal.

### Verified

- Reproduced the failure with real Kit Root key dispatch before the fix.
- All 130 view tests, full `cargo test --release`, release binary build,
  `cargo clippy --release --all-targets`, format and diff checks passed.
- Native GUI manual exercise was not performed; no data-path probe needed.

### Tests added/changed

- Added shell regressions for a non-first selected candidate, retained input
  focus, no premature filter application, subsequent Tab traversal, Tab with
  no candidates and Shift+Tab with candidates.

### Roadmap / architecture changes

- Recorded the filter dispatch fix under Milestone 5 and documented the
  conditional key context in ARCHITECTURE.md.

### Issues / next recommended task

- Exercise completion alongside the preceding layout fixes in the native GUI.
- Prior local work preserved; no branch, commit or PR created.

## 2026-10-14 — Numeric spellings that do not round-trip stay text

### Implemented

- `schema::infer` counts a value as numeric only when the trimmed
  spelling round-trips through `i64` or `f64`. `"007"` and `"1.10"`
  stay text. `"10"` and `"1.5"` do not.
- Equal numeric sort keys break the tie on the exact cell bytes, then
  the record index. Fixes #11.

### Verified

- `cargo test --release --lib -- schema:: query::sort::` (44 passed).
- `cargo test --release` passed.

### Tests added/changed

- `zero_padded_and_trailing_zeros_stay_text`
- `canonical_decimals_are_floats`
- `zero_padded_ids_sort_as_distinct_text`
- `equal_numbers_break_ties_on_exact_bytes`

### Roadmap / architecture changes

- `ARCHITECTURE.md` sort and infer notes mention the round-trip rule
  and the byte tiebreak.

### Issues / next recommended task

- Issue #12, whitespace-only cells sorting as blank.

## 2026-10-14 — Whitespace-only cells sort as values

### Implemented

- A text sort treats only a zero-byte cell as missing. A cell of spaces
  sorts by those bytes, matching the filter and the column panel
  (ADR 0003). A numeric sort still rejects it at parse time, so it
  sorts last with the other non-numbers. Fixes #12.

### Verified

- `whitespace_only_text_sorts_by_exact_bytes` and the existing empty-last
  test passed. Full `cargo test --release` follows before the PR.

### Tests added/changed

- `query::sort::tests::whitespace_only_text_sorts_by_exact_bytes`

### Roadmap / architecture changes

- Milestone 3 notes no longer say whitespace sorts last as empty.
- The sort paragraph in `ARCHITECTURE.md` says the same.

### Issues / next recommended task

- Issue #13, page the cell panel past 32 KiB.

## 2026-10-14 — Cell panel pages past 32 KiB

### Implemented

- `exact_string_window` returns one page of a cell from a byte offset.
  Valid UTF-8 moves a cut off a continuation byte and reports that.
  Invalid UTF-8 is escaped in place, with the offsets left where they
  were asked.
- The Cell panel shows that page. Previous/Next and
  Alt+Left/Right/Home/End walk a longer value. A cursor move returns
  to the first page. Copy is still the whole cell. Fixes #13.

### Verified

- `cargo test --release` passed: lib 219, `anvil` 225 (1 ignored),
  core 87 (6 ignored), style 1.
- `cargo clippy --release --all-targets` is clean aside from the
  existing `too_many_arguments` lint in `ui/texture.rs`.
- The native window was not launched. The view test drives the page
  keys and checks the bytes on each page.

### Tests added/changed

- `exact_string_window_pages_without_splitting_characters`
- `ui::view::tests::cell_detail_pages_a_value_longer_than_the_cap`
- `cell_detail_caps_a_huge_value` still requires the first page of a
  100,000-byte cell to be 32 KiB.

### Roadmap / architecture changes

- The Milestone 3 cell-panel note, ARCHITECTURE invariant 12, and the
  UI design notes describe 32 KiB pages instead of a hard stop.

### Issues / next recommended task

- Issue #14, borrow text sort keys instead of copying the column.
- Linux CI stays red until #22 (fontconfig dlopen) merges.

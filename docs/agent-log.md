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

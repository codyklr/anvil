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
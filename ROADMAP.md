# Anvil Roadmap

> This roadmap is a living plan. Completed items should reflect the
> actual repository state — *not* the original aspirational
> description. Do not mark a task complete until the implementation
> exists, compiles, and is verified.

Status legend:

- `[x]` — implemented and verified.
- `[ ]` — not started or partially started (see notes).
- `[~]` — partially started; see notes.

Every milestone lists the **goal**, the **user-visible outcome**, the
**implementation tasks**, the **dependencies**, and the **acceptance
criteria**. Each task is small enough to fit inside one agent
iteration.

---

## Milestone 1 — Core Viewer

**Goal.** Open a CSV/TSV/delimited text file or a ZIP archive
containing one, display it as a virtualised grid that scrolls smoothly,
and react appropriately when the archive has multiple data files.

**User-visible outcome.**

- Launching `anvil` shows an empty window with an **Open…** button.
- Dragging a CSV/TSV/PSV onto the window loads it.
- Dragging a ZIP with one data file opens that file automatically.
- Dragging a ZIP with multiple data files shows an in-app picker.
- The grid displays sticky headers, row numbers, and horizontal +
  vertical scrollbars.
- The status bar shows row/column counts, detected delimiter, file
  size, and a warning count.

### Tasks

- [x] Native GPUI window with titlebar (`ui::app::run`).
- [x] File-open dialog via `rfd` (`MainView::pick_file`).
- [x] Direct file mmap + indexing (`source::open_path`,
  `parser::indexing::build`).
- [x] Delimiter / quote / header detection
  (`parser::delimit::detect`).
- [x] Virtualised row rendering (`ui::grid::render_row`,
  `gpui::uniform_list`).
- [x] Sticky column header with horizontal scroll alignment
  (`ui::grid::render_header`).
- [x] Horizontal scrollbar with drag support
  (`MainView::render_h_scrollbar`).
- [x] Status bar with row/column count, delimiter, size, warning count
  (`MainView::render_status`).
- [x] Drag-and-drop of any supported path (`MainView::on_file_drop`).
- [x] ZIP magic-byte sniff (`source::open_path` 4-byte read).
- [x] ZIP entry discovery + classification
  (`source::archive::evaluate_entry`).
- [x] ZIP safety: encryption / unsupported compression /
  `__MACOSX` / `.DS_Store` / hidden / `..` / size cap / ratio cap
  (covered by `source::archive` + tests).
- [x] Auto-open when archive has exactly one usable entry or one
  best-ranked entry (`loader::best_candidate`).
- [x] In-app archive picker overlay when several entries share the
  best rank (`MainView::render_overlay`).
- [x] Extract-to-temp + mmap the resolved entry
  (`ArchiveSource::extract_into_temp`).
- [x] Temp-file cleanup via `Arc<TempDir>` (`OpenedSource.temp_dir`).
- [x] Background open + indexing on GPUI executor
  (`MainView::begin_open`).
- [x] Generation counter to discard obsolete work
  (`MainView.generation`).
- [x] Synthetic 1M-row CSV generator (`bin/gen_fixture.rs`).
- [x] Single-CSV ZIP generator with noise entries (`write_zip`).
- [x] Multi-CSV ZIP generator (`write_multi_zip`).
- [x] Malformed CSV fixture (`gen_fixture`).
- [x] Headless `--probe` mode for CI / measuring (`main.rs::probe`).
- [x] Data-side integration tests (38 tests in `tests/core.rs`).
- [x] `measure_large_fixture` ignored test for indexing throughput.

### Dependencies

- `gpui 0.2.2` (with `font-kit` only), `rfd 0.17`, `memmap2 0.9`,
  `memchr 2`, `zip 2.4` (`deflate`, `bzip2`), `tempfile 3`,
  `thiserror 1`, `crc32fast 1` (dev).

### Acceptance criteria

- `cargo build --release` succeeds.
- `cargo test --release` passes (38 tests).
- `cargo run --release --bin gen_fixture fixtures 1000000` produces
  `large.csv`, `archive.zip`, `multi.zip`, `malformed.csv`.
- `cargo run --release --bin anvil --probe fixtures/large.csv`
  reports 1,000,000 rows, 20 columns, < 1 s on the reference machine.
- `cargo run --release --bin anvil --probe fixtures/archive.zip`
  reports 1,000,000 rows for `data/large.csv`, ~1 s on the reference
  machine.
- `cargo run --release --bin anvil --probe fixtures/multi.zip`
  lists multiple candidate files and prints
  `(multiple files; the GUI would show the archive picker)`.
- The GUI window opens, the file picker works, drag/drop works, the
  archive picker appears for `multi.zip`, and scrolling is smooth on
  the 1M-row fixture.

### Known gaps carried forward into Milestone 2

- No progress UI during long extractions.
- No keyboard shortcut to open files.
- ZIP extraction always writes the full entry to a temp file; no
  streaming.
- Window cannot be closed gracefully from inside the app other than
  by the OS close button.

---

## Milestone 2 — Interaction and Query Basics

**Goal.** Make the grid navigable, copyable, and searchable. None of
this may freeze the UI.

### Tasks

#### Selection model

- [x] Introduce `Selection` type storing `(record_index, column_index)`
  with optional anchor + cursor for ranges.
- [x] Render selected cells with `theme::BG_SELECTED` background.
- [x] Mouse-click selection on the grid canvas.
- [x] Shift-click range selection.
- [ ] Click-and-drag range selection.

#### Keyboard navigation

- [x] Arrow keys move the selection (Up/Down/Left/Right).
- [x] Page Up / Page Down step the selection by `PAGE_STEP` rows.
- [x] Home / End jump the column cursor to the first / last column.
- [ ] Cmd/Ctrl+Home/End jump to first / last row.
- [ ] Wire a dedicated `commands` module (currently handled inline in
  `view.rs::on_grid_key_down`).
- [ ] Tab / Shift-Tab navigation between cells.
- [ ] Cmd/Ctrl+G — go-to-row input box.
- [x] Esc — clear selection / close overlays.
- [x] Cmd/Ctrl+C — copy selection to clipboard.

#### Clipboard

- [x] `clipboard` integration via `arboard` or platform layer.
- [x] Copy single cell.
- [x] Copy row range.
- [x] Copy column range.
- [x] TSV formatting for pasted-into-spreadsheet destinations.

#### Search

- [ ] Cmd/Ctrl+F opens search bar.
- [ ] Search input + options (case-sensitive toggle, column scope).
- [ ] `query::search` module: streaming search over
  `ParsedDocument` returning `Vec<record_index>` (or column-qualified
  results).
- [ ] Background search task with generation counter; cancellation
  when input changes.
- [ ] Match count in status bar.
- [ ] Highlight matches in the visible rows.
- [ ] F3 / Shift-F3 — next / previous match; scroll into view.
- [ ] Column-scoped search (click a header, then Cmd/Ctrl+F).

#### Filter

- [ ] Cmd/Ctrl+L opens filter bar.
- [ ] Filter predicates: equals, not equals, contains, not contains,
  starts with, ends with, empty, non-empty, numeric `>`, `>=`, `<`,
  `<=`.
- [ ] `query::filter` evaluates predicates on background executor and
  returns a `Vec<u64>` of matching record indexes.
- [ ] Grid renders the filtered list rather than the full range.
- [ ] Filtered row count in status bar (`rows (filtered from N)`).
- [ ] Filter cancellation / invalidation by generation counter.

#### Tests

- [ ] Unit tests for `query::search` (needle matching, escape
  handling, case sensitivity, column scoping).
- [ ] Unit tests for `query::filter` predicates against fixtures.
- [ ] Test that opening a new file cancels an in-flight search /
  filter.
- [x] Test selection copy formatting (TSV with CRLF on Windows
  builds, LF on Unix).
- [x] UI regression tests for grid focus / keyboard routing — arrow
  keys reach the grid and Cmd/Ctrl+C routes to the copy handler
  (`src/ui/view.rs`, GPUI `TestAppContext`). These fail if the grid
  stops tracking focus.

### Dependencies

- `arboard = "3"` (text-only, default-features disabled) — added in
  the clipboard slice. The crate is used only by the `anvil` binary;
  `anvil_core` is unaffected.
- `gpui` `test-support` (dev-only) — added with the UI regression
  tests. Enables `TestAppContext`; release builds are unaffected.
- The `Cell<'a>` API may need to expose `to_string_lossy_owned`
  consistently for clipboard/search.

### Acceptance criteria

- Selection, keyboard navigation, copy, search, and filter all
  behave correctly on the 1M-row fixture.
- Search across a 1M-row file completes in < 2 s on the reference
  machine without freezing the UI (verify with a visible animation
  or a frame timer).
- Filter on `country == "US"` against the 1M-row fixture returns
  ~10% of rows and updates the grid in < 2 s.
- `cargo test --release` passes; new tests are added for selection,
  search, and filter.
- Performance regression on opening / scrolling the 1M-row fixture is
  ≤ 10%.

---

## Milestone 3 — Sorting and Column Intelligence

**Goal.** Add a column inspector and a sortable grid, powered by cheap
column-level metadata.

### Tasks

#### Sorting

- [ ] Click a header to sort; click again to reverse; third click
  clears.
- [ ] `query::sort` module: `Vec<u64>` permutation of record indexes,
  built on background executor.
- [ ] Numeric columns sort numerically (Milestone 3's type inference
  drives this); other columns sort lexicographically with a sane
  locale-free ordering.
- [ ] Empty / null values sorted to the end, not scattered.
- [ ] Sorting must not freeze the UI; tested by spinning a 1M-row
  numeric column sort.
- [ ] Sort + filter compose: filter narrows rows, sort orders the
  narrowed set.

#### Type inference

- [ ] `schema::infer` scans each column once and assigns one of:
  integer, float, boolean, text, date-like, datetime-like, UUID-like,
  email-like, URL-like, mostly-null.
- [ ] Inference is heuristic and reports confidence, not a strict
  schema. Mostly-null columns are still text.
- [ ] Inference runs on background executor once per open.

#### Column inspector

- [ ] Right-side panel showing the selected column.
- [ ] Display: column name, inferred type, total rows, non-empty,
  empty/null, distinct count (cardinality), duplicate count, min,
  max, longest, shortest, sample values.
- [ ] Frequent-values table (top N with counts).
- [ ] Exact vs approximate distinction: distinct count is exact when
  the column is small enough to fit a `HashSet`; above a threshold
  it falls back to a HyperLogLog sketch.

#### Tests

- [ ] Type inference tests for each kind with positive and negative
  fixtures.
- [ ] Sort tests: numeric vs lexicographic, empty placement, sort +
  filter composition.
- [ ] Column inspector metrics tests against the generated fixture
  and against an explicit small fixture with known counts.

### Dependencies

- `hashbrown` (optional, for cardinality tracking) — new.
- `ordered-float` (optional) for safe float comparisons — new.

### Acceptance criteria

- Sorting a 1M-row numeric column completes in < 3 s on the
  reference machine without freezing the UI.
- Type inference on the generated fixture labels columns correctly
  (id → integer, amount → float, verified → boolean, email →
  email-like, timestamp → datetime-like, country → text, etc.).
- The column inspector opens for any column, displays the documented
  metrics, and explicitly distinguishes "exact" from "approximate"
  when the distinct-count threshold is crossed.
- `cargo test --release` passes; new tests cover inference and sort.

---

## Milestone 4 — Diagnostics and Data Quality

**Goal.** Surface data-quality problems at navigation speed.

### Tasks

#### In-record diagnostics

- [ ] Inconsistent row width detector with per-row detail.
- [ ] Unterminated-quote navigation.
- [ ] Empty / duplicate header navigation.
- [ ] Detection of unusually long records (existing `WarningKind`
  variant; needs emitting).
- [ ] Navigation jumps the grid to the offending row.

#### Encoding diagnostics

- [ ] Per-cell UTF-8 validity flag exposed by `Cell`.
- [ ] Encoding UI: columns with N non-UTF-8 cells are flagged.
- [ ] (Optional) Detect BOM and report encoding.

#### Archive diagnostics

- [ ] Surface skipped archive entries (rejected_reason) in the
  archive picker.
- [ ] Surface decompression-ratio warnings (already emitted as
  errors; make sure they're presented in the UI nicely).
- [ ] Detect encrypted entries explicitly with a friendlier message.

#### Richer filters

- [ ] Compound predicates (`AND` / `OR`).
- [ ] Saved filters.
- [ ] Filter against inferred numeric type (so
  `amount > 1000.0` works as a real comparison).

#### Tests

- [ ] Diagnostics tests with the `malformed.csv` fixture.
- [ ] Filter expression tests (compound, numeric, regex maybe).

### Dependencies

- Same as Milestones 2 and 3.

### Acceptance criteria

- Opening `fixtures/malformed.csv` shows the warning count and lets
  the user click through to each offending row.
- Filters compose; numeric comparisons work.
- `cargo test --release` passes.

---

## Milestone 5 — Performance and UX Hardening

**Goal.** Production-quality polish. No new feature class; instead,
make what exists feel finished.

### Tasks

#### Performance

- [ ] Profile indexing on a multi-hundred-MiB file and capture a
  baseline number in `ARCHITECTURE.md`.
- [ ] Profile scrolling with `cargo bench` or by capturing frame
  times.
- [ ] Add an LRU cache for cell text in the visible range (measure
  before / after).
- [ ] Reduce per-cell allocation in `render_row` by preferring the
  borrowed `Cell<'a>` path; only convert to `String` when
  unavoidable.
- [ ] Memory usage measured and reported per fixture.
- [ ] Cancellation robustness: document the contract that
  generation-based cancellation is sufficient.

#### Test coverage

- [ ] Extend the GPUI `TestAppContext` suite beyond grid focus:
  drag/drop, the archive picker overlay, and toolbar clicks.

#### UX

- [ ] Recent files list (persisted to `$XDG_CONFIG_HOME/anvil/`).
- [ ] Loading progress UI for extraction and indexing.
- [ ] Column resize: drag column boundary in the header.
- [ ] Keyboard shortcut cheatsheet (Cmd/Ctrl+?).
- [ ] Theme toggle (light mode).
- [ ] Optional distro packaging (`brew`, `apt`, `.deb`, etc.).

#### Diagnostics UI

- [ ] Diagnostics panel (side panel or modal) listing every warning
  with one-click navigation.
- [ ] "Show all warnings" mode that highlights every offending cell
  in the visible range.

### Dependencies

- `dirs` (for `XDG_CONFIG_HOME`) — new.
- Anything required for column resize (probably already covered by
  GPUI primitives).

### Acceptance criteria

- A profiling pass produces a written record in
  `/docs/agent-log.md` (or a new `/docs/benchmarks.md`) with
  measurements for at least one large fixture.
- Recent files persists across launches.
- Column resize and theme toggle both work.
- `cargo test --release` passes; performance numbers have not
  regressed vs. the Milestone 1 baseline by more than 10%.

---

## Future Ideas

Not committed to any milestone. Listed so the next agent can decide
whether to elevate one.

- **Additional archive formats** — `gz`, `tar.gz`, `7z`, `tar.zst`,
  `parquet` (parquet would require a new source path that bypasses
  the parser).
- **Multiple datasets / tabs** — keep several documents open at once
  and switch with a tab bar. Requires revisiting `MainView`'s
  ownership model.
- **Dataset comparison** — diff two files by `(record_index, column)`
  with a side-by-side view.
- **Export filtered / sorted rows** — write the current view to a
  new CSV/TSV/JSON.
- **Saved filters / sorts / layouts** — named views the user can
  re-apply.
- **SQL-like queries** — a small expression evaluator on top of the
  filter language; possibly using `datafusion`-style expressions.
- **Schema mapping** — load a second file as a schema and reconcile.
- **Relationship inference** — column-overlap heuristic across two
  files.
- **Integration with other internal tooling** — for instance, reading
  audit logs from Anvil launches into a shared dashboard.
- **Light mode + theme persistence** — already partly listed under
  Milestone 5; promoted here for emphasis.
- **Column-level operations** — group-by / aggregation (Milestone 3
  lays the foundation but does not include aggregation).
- **Bookmarks / pinned rows** — let the user mark rows for later
  recall.

---

## Acceptance discipline

A task is not done when code is written. It is done when:

1. Implementation exists in the repository.
2. `cargo build --release` succeeds.
3. `cargo test --release` passes (including any new tests added).
4. Behaviour has been exercised — either by a test, by `--probe`, or
   by manually launching the GUI.
5. Documentation (`ROADMAP.md`, `ARCHITECTURE.md`, `AGENTS.md`,
   `docs/agent-log.md`) has been updated to reflect the new state.
6. The agent log handoff entry is written.

If part of this cannot be verified, the agent log entry must state
exactly what was and was not verified.
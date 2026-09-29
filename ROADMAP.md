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
  `(multiple files; the GUI opens every usable entry as a tab)`.
  (The probe prints the loader's `NeedArchiveChoice` result; the UI
  now turns that into one tab per entry rather than a modal picker.)
- The GUI window opens, the file picker works, drag/drop works, every
  usable `multi.zip` entry appears as a tab, and scrolling is smooth on
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
- [x] Click-and-drag range selection.
- [x] Drag auto-scroll: holding the drag past the top/bottom edge
  scrolls the list and grows the selection, including while the
  pointer is held still.

#### Keyboard navigation

- [x] Arrow keys move the selection (Up/Down/Left/Right).
- [x] Page Up / Page Down step the selection by `PAGE_STEP` rows.
- [x] Home / End jump the column cursor to the first / last column.
- [ ] Cmd/Ctrl+Home/End jump to first / last row.
- [x] Wire a dedicated `commands` module for keystroke-to-action
  mapping (`ui/commands.rs`); `view.rs::on_grid_key_down` delegates
  execution to `MainView::run_action`. GPUI-native `actions!` /
  `cx.bind_keys` rebinding remains a possible later enhancement.
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
- [x] Visual copy confirmation: the copied cells flash green for
  ~450 ms (`theme::BG_COPIED`) in addition to the status-bar message.

#### Multi-document tabs

- [x] `DocumentSnapshot` per-document state (document, selection,
  scroll, search, copy flash) plus `documents: Vec<DocumentSnapshot>`
  and `active: Option<usize>` on `MainView`. Live per-document
  fields mirror the active snapshot so render/test code keeps one
  set of names.
- [x] Horizontal tab strip above the grid; click to switch, × to
  close, zero-height when no documents are open.
- [x] Multi-file drop: every dropped path becomes its own tab.
- [x] `switch_to` / `close_active_at` with correct active-index
  adjustment for closing active / earlier / last tabs.
- [x] Opening a new file no longer replaces the existing document;
  it adds a tab.
- [x] Every usable ZIP entry becomes a tab immediately on open
  (`install_archive_tabs`); the ZIP central directory is enough to
  build the strip without decompressing anything. The entry the
  loader pre-opened is marked ready; the rest are pending.
- [x] Pending tabs extract lazily: `ensure_active_loaded` runs
  `load_entry` on first activation, `apply_lazy_result` stores the
  document (looked up by stable tab `id`) and rebuilds the model
  only if the tab is still active. A detached task means switching
  away does not cancel the load or leave the tab stuck.
- [x] Multi-entry ZIPs no longer force a modal picker. The picker
  remains reachable from the toolbar's **Archive…** button as an
  entry list; clicking an entry switches to its tab
  (`switch_to_entry`).
- [x] Errors from background open tasks (including a failed lazy
  extraction, which drops the tab) surface in the status bar and do
  not tear down the other tabs.
- [x] Left-hand file-manager panel listing every open document,
  grouped by archive (`group_documents`, keyed by `Arc` identity so
  same-named archives stay separate). Click to switch, × to close;
  loaded/pending dot per row. Toggled by Cmd/Ctrl+B, the toolbar
  **Files** button, and the panel's own × .
- [x] Tab strip scrolls horizontally instead of crushing tabs when
  the list is long (`overflow_x_scroll` + `tab_scroll`).
- [ ] Dockable / resizable / right-side panel, and a truly nested
  tree (directories inside archives). The data model already
  supports it; only the presentation is missing.
- [ ] Tab reordering and a tooltip for truncated labels.
- [ ] Remember recently-closed tabs.

#### Search

- [x] Cmd/Ctrl+F opens search bar.
- [ ] Search input + options (case-sensitive toggle, column scope).
- [x] `query::search` module: streaming search over
  `ParsedDocument` returning `Vec<record_index>` (or column-qualified
  results).
- [x] Background search task with generation counter; cancellation
  when input changes.
- [x] Match count in status bar.
- [x] Highlight matches in the visible rows.
- [x] F3 / Shift-F3 — next / previous match; scroll into view.
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

- [x] Unit tests for `query::search` (needle matching, escape
  handling, case sensitivity, column scoping).
- [ ] Unit tests for `query::filter` predicates against fixtures.
- [x] Test that opening a new file cancels an in-flight search /
  filter.
- [x] Test selection copy formatting (TSV with CRLF on Windows
  builds, LF on Unix).
- [x] UI regression tests for grid focus / keyboard routing — arrow
  keys reach the grid and Cmd/Ctrl+C routes to the copy handler
  (`src/ui/view.rs`, GPUI `TestAppContext`). These fail if the grid
  stops tracking focus.
- [x] UI regression tests for the search bar (Cmd/Ctrl+F opens the
  bar; typed characters append to the query; Backspace deletes; Enter
  / Shift+Enter cycle matches; Esc closes; F3 / Shift-F3 cycle matches
  without opening the bar; opening a new document clears the search
  state).
- [x] UI regression tests for multi-document tabs (`apply(Ready)`
  creates a tab; two sequential opens create two tabs; errors with
  an open tab keep it; tab switch restores per-document selection /
  scroll; closing the active / earlier / last tab adjusts `active`;
  every usable ZIP entry becomes a tab synchronously; each pending
  tab lazy-loads on activation; a pre-opened favourite is the ready
  active tab).
- [x] UI regression tests for the file panel (Cmd/Ctrl+B toggles it;
  `group_documents` nests ZIP entries under their archive keyed by
  `Arc` identity; panel switch / close reuse the tab lifecycle).

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

#### Cross-platform support

- [ ] Add a CI build/test matrix for macOS, Windows, and Linux:
  `cargo fmt --check`, `cargo build --release`, `cargo test --release`,
  and `anvil --probe fixtures/large.csv`.
- [ ] Exercise the native GUI on all three target OSes: open dialog,
  drag/drop, archive picker, grid scrolling, keyboard shortcuts,
  clipboard copy, and save/open paths where applicable.
- [ ] Verify platform-specific clipboard behavior (Cmd vs Ctrl and
  LF vs CRLF), native file dialogs, mmap handling, and GPUI backend
  requirements; document any supported Linux display/backend limits.

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
- macOS, Windows, and Linux each pass the CI build/test/probe matrix;
  the GUI's core open, inspect, keyboard, and clipboard workflows have
  been exercised on each target.

---

## Future Ideas

Not committed to any milestone. Listed so the next agent can decide
whether to elevate one.

- **Additional archive formats** — `gz`, `tar.gz`, `7z`, `tar.zst`,
  `parquet` (parquet would require a new source path that bypasses
  the parser).
- **Dockable / resizable side panel** — a left file panel landed in
  Milestone 2 (see the Multi-document tabs section). Making it
  resizable, movable to the right, or collapsible to an icon rail,
  and growing it into a truly nested tree, can reuse
  `DocumentSnapshot` and `group_documents` unchanged.
- **Dataset comparison** — diff two files by `(record_index, column)`
  with a side-by-side view.
- **Configurable copy format** — give Cmd/Ctrl+C a chosen output
  format rather than hard-coded TSV. Candidate formats: TSV (default,
  spreadsheet paste), CSV (RFC 4180 quoting), Markdown table, JSON
  (array of objects keyed by header), and plain text (values only).
  Surface it as a toolbar dropdown / settings menu, with a one-off
  "Copy as ▸" override (e.g. Cmd/Ctrl+Shift+C opens the format menu).
  Implementation: replace the delimiter assumption in
  `Selection::copy_text_with` with a `CopyFormat` enum and a small
  formatter per variant; include the header row for CSV / Markdown /
  JSON when headers exist. Persist the preference (ties into the
  config-file work in Milestone 5). The copy-confirmation flash and
  status message should stay format-agnostic.
- **Select-all (Cmd/Ctrl+A)** — select the whole dataset (all records
  × all columns). Cheap with the current rectangular model: anchor
  `(0,0)`, cursor `(record_count-1, column_count-1)`. Land this before
  non-contiguous selection. Consider row/column selection from the
  gutter and header clicks too.
- **Non-contiguous multi-select** — Cmd/Ctrl-click to add separate
  cells or blocks (and Cmd/Ctrl+drag to add a rectangle) to a single
  selection. Today `Selection` is one anchor→cursor rectangle, so this
  needs the model to hold an ordered set of rectangles plus the
  existing cursor for keyboard navigation. Copy would emit the union
  in row-major order, separated per block (blank line between blocks)
  the way spreadsheets do.
- **Export selection / view to a file** — write the current selection
  (or the whole filtered / sorted view) to a new CSV/TSV/JSON/Markdown
  file via `rfd`'s save dialog, reusing the `CopyFormat` formatter
  from the configurable-copy idea above rather than duplicating it.
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
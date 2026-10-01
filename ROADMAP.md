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
- [x] Cmd/Ctrl+Home/End jump to first / last row. Plain Home / End
  jump to the first / last column. Implemented via
  `Action::MoveCursor` with an `i64::MIN` / `i64::MAX` row delta;
  `Selection::move_cursor` now uses saturating arithmetic so those
  extremes cannot overflow.
- [x] Wire a dedicated `commands` module for keystroke-to-action
  mapping (`ui/commands.rs`); `view.rs::on_grid_key_down` delegates
  execution to `MainView::run_action`. GPUI-native `actions!` /
  `cx.bind_keys` rebinding remains a possible later enhancement.
- [x] Tab / Shift-Tab navigation between cells. `Selection::move_cell`
  advances in row-major order, wrapping across rows and around the
  document ends; with a filter active the walk covers visible rows
  only.
- [x] Cmd/Ctrl+G — go-to-row input box. `ui/view/goto.rs` adds a
  one-line bar over the shared `TextEdit`. Enter resolves a 1-based
  display row, clamps it to `[1, visible rows]`, maps it through the
  filtered/sorted view to a record index, moves the cursor there
  (keeping the current column) and scrolls it to the top; Esc closes
  the bar. Reuses the same deferred-focus and mouse caret patterns as
  the search and filter bars.
- [x] Cmd/Ctrl+A — select the whole view. `Selection::select_all`
  anchors at `(0, 0)` and puts the cursor at the last row / last
  column. Under an active filter or sort the copy path walks the
  visible display order, so hidden records are neither highlighted nor
  copied. The command does not reach the filter / search / Ask inputs,
  which handle Cmd/Ctrl+A themselves.
- [x] Cmd/Ctrl+O — open a file through the toolbar **Open…** path.
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
- [x] Search input + options. The bar has an `Aa` toggle for
  case-sensitive (byte-exact) search and a column-scope chip showing
  the scoped column's header name with a × to clear it. Both options
  persist per tab and re-run an in-flight search when toggled.
- [x] `query::search` module: streaming search over
  `ParsedDocument` returning `Vec<record_index>` (or column-qualified
  results).
- [x] Background search task with generation counter; cancellation
  when input changes.
- [x] Match count in status bar.
- [x] Highlight matches in the visible rows.
- [x] F3 / Shift-F3 — next / previous match; scroll into view.
- [x] Column-scoped search: **Alt/Option-click** a column header to
  scope the search to that column; click again to clear it. (A plain
  header click sorts.) The header stays highlighted while scoped, so
  the state is visible even when the bar is closed, and Enter / F3 land
  the cursor in the scoped column.
- [x] Navigating to a match keeps it on screen on both axes. The
  vertical scroll maps the record through the filtered/sorted display
  order (previously the raw record index was used as a display
  position), and the grid scrolls horizontally so the whole matching
  cell is visible — the scoped column, or the first matching column for
  a whole-record search. Arrow / Tab navigation also scrolls the cursor
  column into view. The grid's `viewport_w` now excludes the open
  file panel, so the scroll target is not clipped behind it.

#### Filter

- [x] Cmd/Ctrl+L opens filter bar.
- [x] `query::filter` module: `Filter` / `FilterOp`, a one-line
  expression parser (`<column> <op> [value]`, header name / index /
  `#N`), and `apply` returning the matching record indexes in order.
  Operators: `==`, `!=`, `contains`, `!contains`, `startswith`,
  `endswith`, `empty`, `notempty`, numeric `>`, `>=`, `<`, `<=`.
  Text operators are ASCII case-insensitive; ordering operators skip
  non-numeric cells. Multiple filters combine with AND.
- [x] Row-scope `any` / `all` pseudo-columns (`any empty`,
  `all notempty`, `any contains X`) as `FilterExpr::AnyColumn` /
  `AllColumn`. Evaluated by looping every column per row over one
  prepared predicate with short-circuiting, rendered by `describe`
  (round-trip tested), and offered by autocomplete. `all notempty`
  treats missing cells as empty.
- [x] Application runs on the background executor; a generation
  counter discards obsolete results when the predicate changes.
- [x] Grid renders the filtered list rather than the full range; a
  display-position -> record-index mapping is applied per row.
- [x] Filtered row count in status bar (`rows (filtered from N)`).
- [x] Vertical arrow navigation walks the visible rows, skipping
  hidden records.
- [x] The filter input is a real editable text field: click to place
  the caret, click-drag / Shift+click to select (highlighted), arrow /
  Home / End / word movement, Shift-extension, Cmd/Ctrl+A, and
  Backspace/Delete over the selection. Backed by the GPUI-free
  `ui::text_edit::TextEdit` model; pointer x is mapped to a byte index
  through the shaped text layout.
- [x] The search bar uses the same editable input as the filter bar
  (`TextEdit`): caret movement, Shift-selection, Cmd/Ctrl+A, and
  mouse click/drag, with the live search re-running after edits.
- [x] A richer expression language (`AND` / `OR`, parentheses); the
  data-side already composes a `Vec<Filter>` with AND, so this is a
  parser and UI extension. Landed as `FilterExpr` / `parse_expr` (see
  the Milestone 4 richer-filters section).
- [x] Autocomplete suggestions while typing. A context-aware popup
  (`query::suggest` + `ui/view/filter.rs`) offers columns, operators,
  `AND`/`OR`, and bounded sampled values for the value slot. Up/Down
  navigate, Enter/Tab or a mouse click fill the input, and Esc closes
  the popup before the bar. Value samples are cached per column and
  invalidated when the document changes.
- [ ] Saved filters / filter chips.

#### Tests

- [x] Unit tests for `query::search` (needle matching, escape
  handling, case sensitivity, column scoping).
- [x] Unit tests for `query::filter` predicates against fixtures
  (resolution, every operator, case folding, numeric parsing,
  missing cells, AND composition, parse errors).
- [x] Test that opening a new file cancels an in-flight search /
  filter.
- [x] Test selection copy formatting (TSV with CRLF on Windows
  builds, LF on Unix).
- [x] Unit + UI tests for Cmd/Ctrl+Home/End row jumps and
  Tab / Shift-Tab cell navigation, including the filtered path.
- [x] Unit tests for the `TextEdit` model (insert / replace selection /
  backspace / delete / movement / selection / multibyte boundaries)
  and UI tests for filter- and search-field caret movement,
  select-all + typing, mouse click / drag selection, and live
  re-search after a mid-string insert.
- [x] UI regression tests for grid focus / keyboard routing — arrow
  keys reach the grid and Cmd/Ctrl+C routes to the copy handler
  (`src/ui/view/tests.rs`, GPUI `TestAppContext`). These fail if the
  grid stops tracking focus.
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

- [x] Click a header to sort; click again to reverse; third click
  clears. (Alt/Option-click sets the search column scope instead.)
  The header shows a ▲ / ▼ direction arrow and the status bar shows
  `sorted by <column> ▲`.
- [x] `query::sort` module: `Vec<u64>` permutation of record indexes,
  built on the background executor. `apply(doc, rows, spec)` sorts the
  supplied view (filtered set or all records) and returns a new
  permutation; a generation counter discards stale results.
- [x] Numeric columns sort numerically (type inference drives this);
  other columns sort case-insensitively on trimmed bytes with a
  locale-free ordering. The sort is stable.
- [x] Empty / non-numeric values sorted to the end, for both
  directions.
- [x] Sorting must not freeze the UI; tested by spinning a 1M-row
  numeric column sort (**126 ms** for `amount`, well under 3 s).
- [x] Sort + filter compose: `kick_sort` builds its base from
  `filter_rows`, and a filter change re-runs the sort.

#### Type inference

- [~] `schema::infer` samples each column once, bounded to 2048 rows,
  and assigns integer / float / boolean / text / empty. The richer
  date-like / datetime-like / UUID-like / email-like / URL-like
  classifications and a full-column scan are follow-ups. On the
  generated fixture it labels `id`/`user_id` integer, `amount`/`risk_score`
  float, `is_verified` boolean, and the rest text.
- [x] Inference is heuristic and reports confidence
  (`ColumnProfile::numeric_fraction`), not a strict schema. All-empty
  columns are `Empty`; a column at the 0.9 numeric threshold is
  numeric.
- [~] Inference runs on the background executor. The sorter computes
  it inside the sort task, and the column inspector computes it inside
  the stats task; caching the profiles on the document at open is still
  an optional follow-up.

#### Column inspector

- [x] Right-side panel showing the selected column (Cmd/Ctrl+I and the
  toolbar **Inspector** button; `×` closes it). It follows the selection
  and rescans only when the column changes.
- [x] Display: column name, inferred type, total rows, non-empty,
  empty/null, distinct count (cardinality), duplicate count, min,
  max, longest, shortest, sample values.
- [x] Frequent-values table (top N with counts).
- [x] Exact vs approximate distinction: distinct is exact while the
  value set fits in a `HashMap`; above `max_exact_distinct` (100k) it
  switches to a dependency-free HyperLogLog sketch (1024 registers,
  with the small-range linear-counting correction) and reports
  `DistinctCount::Approximate`.
- [x] The inspector scans the whole document, not the filtered/sorted
  view. Iteration 8d: `schema::stats::stats_rows` scans just the
  supplied record indexes, and the UI passes `view_rows()` when a filter
  or sort is active (rescanning on a view change via
  `inspector_view_key`). The panel labels the scope
  ("scoped to current view (N of M rows)" / "whole document").

#### Tests

- [x] Type inference tests for each implemented kind with positive
  and negative fixtures (integer, float, boolean, text, empty,
  numeric-threshold boundary, header names).
- [x] Sort tests: numeric vs lexicographic, empty placement, sort +
  filter composition, stability, non-numeric last.
- [x] Column inspector metrics tests against explicit small fixtures
  with known counts (empties, distinct/duplicates, numeric vs text
  min/max, frequent ranking, samples, the exact→approximate
  threshold, headerless naming). A generated-fixture stats assertion
  is still optional.

### Dependencies

- `hashbrown` (optional, for cardinality tracking) — **not** added:
  the exact phase uses `std::collections::HashMap` and the approximate
  phase is a small hand-rolled HyperLogLog.
- `ordered-float` (optional) for safe float comparisons — **not**
  added: `f64::total_cmp` (stable since Rust 1.62) gives a total
  order without a dependency.

### Acceptance criteria

- [x] Sorting a 1M-row numeric column completes in < 3 s on the
  reference machine without freezing the UI (126 ms).
- [~] Type inference on the generated fixture labels columns
  correctly for the implemented kinds (`id`/`user_id` integer,
  `amount`/`risk_score` float, `is_verified` boolean, the rest text).
  Email/date/UUID/URL typing is deferred.
- [x] The column inspector opens for any column (Cmd/Ctrl+I / toolbar
  button), displays the documented metrics, and explicitly
  distinguishes "exact" from "approximate" when the distinct-count
  threshold is crossed.
- [x] `cargo test --release` passes; new tests cover inference, sort,
  and the inspector metrics (~266 total).

---

## Milestone 4 — Diagnostics and Data Quality

**Goal.** Surface data-quality problems at navigation speed.

### Tasks

#### In-record diagnostics

- [x] Inconsistent row width detector with per-row detail.
- [x] Unterminated-quote navigation.
- [~] Empty / duplicate header navigation. These are header-level
  warnings with no `record_index`; the diagnostics panel lists them
  but a click reports that there is no row to jump to instead of
  moving the grid.
- [x] Detection of unusually long records (`VeryLongRecord`).
  `parser::indexing` now flags any record longer than 1 MiB in a post
  pass over the finished offset table, so a multi-MB cell or a record
  that swallowed the file after an unterminated quote is reported at
  its own row.
- [x] Navigation jumps the grid to the offending row.
- [x] Diagnostics panel (Cmd/Ctrl+D and the toolbar **Diagnostics**
  button) lists every warning — kind, row, and note — and jumps the
  grid to that record on click. It reads the immutable
  `ParseMeta.warnings` directly, so there is no background task or
  cache. Offending rows carry a warning-coloured `⚠` row-number
  gutter so the jump lands on a visibly flagged row. A record hidden
  by the active filter reports in the status bar rather than silently
  landing on an unrelated row.

#### Encoding diagnostics

- [x] Per-cell UTF-8 validity flag exposed by `Cell` (`Cell::is_utf8`).
- [x] Data-side `encoding::scan`: one pass over the document counting
  non-UTF-8 cells per column and returning an `EncodingReport`
  (columns with counts, total, BOM flag, records scanned). O(file),
  allocation-light, and intended to run on the background executor.
  `--probe` prints the BOM flag and invalid-cell count.
- [x] Encoding UI: opening the diagnostics panel runs `encoding::scan`
  on the background executor (generation-guarded; cached until the
  document changes) and lists the BOM notice and each column with
  non-UTF-8 cells.
- [x] (Optional) Detect BOM and report encoding. `encoding::scan`
  reports `bom`, `--probe` prints it, and the diagnostics panel shows
  a `UTF-8 BOM present` notice. Iteration 8c: the parser now strips a
  leading BOM (via `strip_utf8_bom` / `bom_len`) so it never appears in
  the first header or cell, while the bytes remain in the source and
  the encoding scan still reports the BOM.

#### Header detection

- [x] Improved automatic detection: the first row's per-column
  text/typed shape is compared against the following rows. A header
  vote is text-over-typed (the `name` over `42` shape); a data vote is
  typed-over-typed. A headerless file whose first row matches the data
  shape (`alice,30 / carol,40`) is now recognised. The genuinely
  ambiguous all-text case keeps the local numeric-fraction fallback
  (and is what the override is for).
- [x] `HeaderMode { Auto, Header, Headerless }` on `IndexOptions`;
  `detect_with` forces the interpretation; `detect` / `parse` stay
  Auto for existing callers.
- [x] Toolbar **Headers** control cycles Auto → On → Off. Forcing a
  mode re-indexes the already-open source on the background executor
  (`anvil_core::reindex`, reusing the mmap / extracted temp file, no
  re-open or re-extract), resets stale filter/sort/search state, and
  clamps an out-of-range selection. The mode is per tab.
- [x] Headerless columns are labelled `col0`, `col1`, … consistently
  in the grid header, search scope chip, sort label, column inspector,
  and encoding scan.

#### Archive diagnostics

- [x] Surface skipped archive entries (rejected_reason) in the
  archive picker. The **Archive…** overlay now renders a *skipped
  entries* section listing each rejected entry's name and reason
  (`archive_picker::skipped_entries`), capped at 40 rows with an
  "…and N more" summary so a noisy archive stays readable.
- [x] Surface decompression-ratio warnings (already emitted as
  errors; make sure they're presented in the UI nicely).
  `evaluate_entry` checks the central-directory uncompressed/compressed
  ratio against `MAX_COMPRESSION_RATIO` and rejects a high-ratio entry
  up front with a "suspicious compression ratio (Nx > 200x limit)"
  reason, so it appears in the skipped list instead of failing during
  extraction. The extraction-time guard remains as a backstop.
- [x] Detect encrypted entries explicitly with a friendlier message.
  `evaluate_entry` rejects encrypted candidates with the
  `REJECTED_ENCRYPTED` reason (shown in the picker);
  `extract_into_temp` returns the new `AnvilError::EncryptedEntry`, and
  `loader::load` returns that typed error when an archive has data files
  but every one is password-protected.
- [x] Unsupported compression methods are likewise reported at listing
  rather than only when extraction fails.

#### Richer filters

- [x] Compound predicates (`AND` / `OR`).
  `query::filter::FilterExpr` is an AND/OR tree with parentheses and
  standard precedence (AND binds tighter than OR). `parse_expr` is a
  recursive-descent parser over a small tokenizer; `apply_expr`
  precomputes each predicate once and short-circuits per row. The
  existing single-predicate `parse` / `apply` keep working unchanged.
  Boolean keywords are case-insensitive; a value that would collide with
  one can be quoted (`name contains "AND"`), and multi-word values still
  work unquoted (`city == New York AND amount > 100`).
- Saved filters / filter chips are tracked as a single item under
  Milestone 2, which owns the filter UI. The predicate language in
  this milestone is complete without them.
- [x] Filter against inferred numeric type (so
  `amount > 1000.0` works as a real comparison). Ordering operators
  already parse each cell and the threshold as `f64`, skipping
  non-numeric cells; a decimal-threshold test now locks the behaviour.
  Inference-driven type validation (e.g. date comparisons) is not part
  of this and is covered by the richer-type-classes follow-up.

#### Tests

- [x] Diagnostics tests: the unterminated-quote `record_index` points
  at the partial record (regression for the off-by-one), the panel
  toggles via Cmd/Ctrl+D / the toolbar, a row warning moves the
  cursor to that record, a warning hidden by the active filter
  reports instead of moving, and a header-level warning reports that
  there is no row.
- [x] Header detection tests: the text/typed vote cases (header vs
  headerless), the ambiguous all-text fallback, forced `Header` /
  `Headerless` modes, `reindex` reusing the opened source, the
  per-tab toolbar cycle, selection clamping when the row count shrinks,
  and the `colN` labels.
- [x] Filter expression tests: 9 unit tests for the compound grammar
  (precedence, parentheses, multi-word and quoted values, keyword case
  folding, parse errors, single-predicate equivalence, OR
  dedup/short-circuit, decimal thresholds) plus a UI test that runs a
  compound expression through the filter bar. Regex matching remains a
  possible addition.

### Dependencies

- Same as Milestones 2 and 3.

### Acceptance criteria

- Opening `fixtures/malformed.csv` shows the warning count and lets
  the user click through to each offending row.
- A headerless file is detected when its first row matches the data
  shape, and the toolbar **Headers** control can force the
  interpretation either way; headerless columns show `col0`, `col1`, ….
- Filters compose; numeric comparisons work.
- `cargo test --release` passes.

---

## Milestone 5 — Performance and UX Hardening

**Goal.** Production-quality polish. No new feature class; instead,
make what exists feel finished.

### Tasks

#### Performance

- [x] Profile indexing on a multi-hundred-MiB file and capture a
  baseline number in `ARCHITECTURE.md`. Iteration 3 measured a 498.3
  MiB / 3,000,000-row fixture at 747 ms (~667 MiB/s) and a 165.4 MiB /
  1,000,000-row fixture at 249 ms, and recorded the full table in
  `docs/benchmarks.md`, referenced from `ARCHITECTURE.md`.
- [~] Profile scrolling with `cargo bench` or by capturing frame
  times. No GPU frame time was captured because the GUI cannot be
  driven in the headless reference environment. A CPU-only proxy for
  the grid's per-cell text work (1,000 frames of 40x20 cells at
  44.4 ns/cell) is recorded in `docs/benchmarks.md`; a real frame timer
  on a desktop is still open.
- [x] Add an LRU cache for cell text in the visible range (measure
  before / after). Iteration 4 added a bounded (4096-entry)
  `(record, column) -> SharedString` cache on `GridModel`. The
  `measure_viewport_cell_text` proxy went from 81.9 ns/cell
  (two-copy form) to 22.6 ns/cell, i.e. 0.066 -> 0.018 ms/frame; the
  cache is a clear-evict FIFO rather than a strict LRU, which is
  enough because a scroll step only invalidates the entering rows.
- [x] Reduce per-cell allocation in `render_row` by preferring the
  borrowed `Cell<'a>` path; only convert to `String` when
  unavoidable. `render_row` now builds `SharedString` via
  `SharedString::new(exact_string(bytes).as_ref())` (one `Arc<str>`
  copy) and serves it from the bounded cache, instead of
  `SharedString::from(text.to_string())` (intermediate `String` plus
  the `Arc` copy). Byte-faithful display is unchanged, including the
  escaped form for invalid UTF-8, and is covered by
  `cell_shared_string_is_byte_faithful`.
- [x] Memory usage measured and reported per fixture.
  `docs/benchmarks.md` reports peak RSS for every fixture, and notes
  that the file-backed mmap pages dominate it while the durable index
  is `record_count x 8` bytes.
- [ ] Cancellation robustness: document the contract that
  generation-based cancellation is sufficient.

#### Cross-platform support

- [~] Add a CI build/test matrix for macOS, Windows, and Linux:
  `cargo fmt --check`, `cargo build --release`, `cargo test --release`,
  and `anvil --probe fixtures/large.csv`. `.github/workflows/ci.yml`
  now defines the three-OS matrix and the Linux dependency install
  step, and every one of its commands was run locally on macOS. The
  workflow has only been validated for syntax; no real GitHub Actions
  run has executed yet, and the Linux and Windows legs are unverified.
- [ ] Enable a real GPUI window backend on Linux. The release build
  uses `gpui` with `default-features = false, features = ["font-kit"]`,
  so neither `x11` nor `wayland` is compiled and
  `gpui::current_platform` always returns the headless client on
  Linux: the binary builds and `--probe` works, but the GUI cannot
  open. Add per-target backend features and the matching native
  packages, then verify the window on a Linux runner.
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
- [x] Loading progress UI for extraction and indexing. `anvil_core`
  reports determinate `LoadProgress` (phase + bytes done/total) through
  an optional `ProgressSink` from both ZIP decompression and the index
  walk; extraction knows the uncompressed size from the central
  directory and the index knows the mmap length, so both bars are
  determinate. The UI stores samples in an atomic `ProgressTracker`
  written by the background task and polled on a 50 ms timer; the
  poller stops when the tracker is marked finished or a newer load
  bumps the progress epoch, so obsolete updates are discarded. An
  unknown total falls back to an indeterminate stub. The indexer's
  per-byte check is behind a const generic, so the no-progress path is
  unchanged (251 ms on the 1M fixture).
- [x] Column resize: drag column boundary in the header. Each header
  cell carries a thin right-edge handle; pressing it starts a drag and
  a global mouse-move rebuilds `ColumnLayout` (`resized` clamps to
  `[MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH]` and recomputes offsets and
  total, so header, body and the horizontal scrollbar stay aligned).
  Widths are stored per tab on `DocumentSnapshot` and restored on tab
  switch. Covered by a `ColumnLayout` clamp/offset unit test, a UI drag
  test, and a tab-switch restore test.
- [ ] Keyboard shortcut cheatsheet (Cmd/Ctrl+?).
- [ ] Theme toggle (light mode).
- [ ] Optional distro packaging (`brew`, `apt`, `.deb`, etc.).

#### Data fidelity

- [x] Byte-faithful display (ADR 0003). The column inspector reports
  the exact cell value for `min` / `max` / `longest` / `shortest` /
  `samples` / `frequent` and counts values that differ only in padding
  as distinct; the grid, the clipboard, `--probe` sample rows and the
  column **headers** all show cells that are not valid UTF-8 in a
  reversible escaped form (`\xNN`, with backslashes doubled) instead of
  hiding them or writing U+FFFD. `parser::exact_string` is the single
  implementation and `ParseMeta.headers` is built with it rather than
  `from_utf8_lossy`; regression tests live in `tests/core.rs` and
  `schema::stats`.
- [x] Make `min` / `max` ordering and text sorting compare the exact
  value rather than the trimmed, case-insensitive form, with the exact
  bytes as a deterministic tiebreak, so two values that differ only in
  padding or case never compare as equal. Iteration 8a: text sort keys
  store the cell's exact bytes (no trim, no case folding) with a record
  index tiebreak; `schema::stats::compare_values` uses exact bytes for
  a text column. Numeric sorting and numeric min/max are unchanged, and
  empty / whitespace-only values still sort last.
- [x] Decide whether a whitespace-only cell is empty. Per ADR 0003 the
  byte-faithful answer is "no, it is a value". The filter `empty`
  operator already used `bytes.is_empty()`; the inspector was changed
  to match (`schema::stats` now counts only zero-byte cells as empty).
  Recorded in `ARCHITECTURE.md` and ADR 0003. Text sorting still places
  whitespace-only values last per the 8a "empty last" rule.
- [x] Add an affordance to reveal the full value of a cell that the grid
  has elided, so a user never needs to copy a truncated-looking cell to
  read it. Cmd/Ctrl+Enter toggles a right-hand **Cell** panel showing
  the cursor cell's exact (escaped) value. The read runs on the
  background executor and is capped at 32 KiB, with the panel stating
  how many bytes were shown when it caps; the panel follows the cursor.

#### Diagnostics UI

- [x] Diagnostics panel (side panel or modal) listing every warning
  with one-click navigation. Delivered in Milestone 4 (Cmd/Ctrl+D,
  the toolbar **Diagnostics** button and `src/ui/view/diagnostics.rs`),
  which carries the full description; this entry is kept as a
  cross-reference so the item is not double-counted.
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

## Milestone 6: Natural-language Ask (opt-in)

**Goal.** Let the user type a phrase such as "US orders over 100" and
get a filter expression, using deterministic rules first and an optional
local model only when the rules cannot cleanly resolve the phrase. See
`docs/adr/0001-natural-language-ask.md`.

**User-visible outcome.**

- `Cmd/Ctrl+Shift+L` opens an **Ask** bar.
- Submitting a phrase previews the constructed filter expression; the
  user accepts it into the normal filter bar (where it can be edited) or
  cancels.
- If the rules cannot fully parse the phrase, the unmatched spans are
  shown rather than guessed.
- If an optional local model is configured, it is consulted only when the
  rules fail; the default build never contacts anything.

### Tasks

#### Prerequisites
- [ ] `FilterExpr::Not`, plus `parse_expr` support for `NOT` / `not` and
  `apply_expr` evaluation; display in the filter bar. Deferred: Tier 1
  handles negation only through negating operators (`is not`,
  `not contains`, `not empty`). Row-scope `any` / `all` landed without
  a general `NOT`.
- [~] Column alias map. `NlSchema::aliases` is accepted and tested, so
  the parser is alias-aware; a user-configurable alias file is still
  open.

#### Tier 1 rules parser (`anvil_core::query::nl`, GPUI-free)
- [x] Tokenizer (whitespace, quoted spans, trailing commas as `and`).
- [x] Operator lexicon ("is", "over", "under", "at least",
  "contains", "starts with", "empty", "not empty", symbols, ...).
- [x] Column resolution: headers, `colN`, aliases, multi-word and quoted
  names. Fuzzy/prefix matching is still open.
- [x] Value binding: longest sampled value first, then numeric literals,
  then a low-confidence raw fallback.
- [x] Conjunctions: "and" -> `And`, "or" -> `Or`; a bare value after a
  conjunction reuses the previous column (`status is failed or`
  `refunded`). General `not` waits on `FilterExpr::Not`.
- [x] `NlResult` returns the `FilterExpr`, unmatched spans, and a
  `clean` flag for the escalation contract.
- [x] 16 unit tests: a phrase table plus ambiguous and unparseable
  phrases.

#### Ask bar (Cmd/Ctrl+Shift+L)
- [x] New bar reusing `TextEdit`; live preview of the parsed expression;
  accept into the filter bar (which then runs the existing apply path);
  `×` / Esc to cancel.
- [x] Show unmatched spans when the parse is incomplete.
- [~] Never auto-apply; Enter accepts only a clean interpretation whose
  preview round-trips through the filter parser. Regression verification
  is in progress; focus returns to the grid after cancel.

#### Tier 1.5 extended rules (default Tier 2)
- [x] Filler-word normalization ("show me", "list all", "rows where", ...).
- [x] Fuzzy column matching (single-token edit distance, ambiguity-safe).
- [x] `between A and B` -> `>= A AND <= B`.
- [x] More operator synonyms ("no more than", "at minimum",
  "matches", "like", ...).
- [ ] Relative dates ("last month", "this year") expanded against a
  reference date.
- [ ] A user-configurable synonym / alias file.

#### Tier 2 optional embedded model (`nl-model`, off by default)
- [x] `NlBackend` trait plus `interpret`, which always re-parses and
  schema-validates a backend's output via `parse_expr`. An external
  `ModelBackend` exists as a power-user override but requires a
  pre-installed command, so it is not the default.
- [x] Embedded inference backend (`src/nlmodel.rs`): `candle-core` +
  `candle-transformers` + `tokenizers` + `ureq`, behind the
  off-by-default `nl-model` feature. Loads a quantized Llama or Qwen2
  GGUF (default Qwen2.5-Coder-1.5B-Instruct Q4_K_M, overridable with
  `ANVIL_NL_MODEL_*` / `ANVIL_NL_TOKENIZER_*`), generates greedily, and
  validates the candidate with `validate_nl_candidate`.
- [~] Download UX: `ureq` fetches and caches the model on the first
  Ctrl/Cmd+Enter. A size/consent dialog, progress bar, and cancel are
  still open.
- [x] Escalate only when Tier 1 and Tier 1.5 do not cleanly resolve and
  the backend is available (Ctrl/Cmd+Enter); otherwise show the partial
  parse.

#### Native GLiNER extraction backend (current implementation slice)

- [~] Evaluate GLiNER2.5 Small through the community `gliner-rs` Candle
  implementation. Verify extraction accuracy and dependency cost before
  adopting it; Python and a separate inference runtime are out of scope.
- [~] Optional in-process backend with bounded requests, background inference,
  and actionable model errors. Keep the model loaded between requests.
- [~] Extract and validate a single predicate against the active schema;
  reject low confidence, multiple values, and common unsupported compound
  requests. General semantic coverage is not guaranteed; review is required.
- [~] Wire the backend to Ask preview/accept; invalidate obsolete results on
  edits and document changes, and prevent acceptance of partial interpretations.
- [x] Resolve indirect columns from sampled values in the GLiNER
  backend: the interpreter now receives the per-column sample callback,
  each column label carries its type, aliases, and up to five sampled
  values, and a single extracted value binds to the unique column whose
  samples contain it (`Which users are from Canada?` ->
  `country == Canada`). Ambiguous matches fail closed; low classifier
  confidence is overridden only by an unambiguous sample match.
- [x] Map row-scope phrases ("at least one column is blank", "every
  column is filled", "any row that has an empty column") to
  `AnyColumn` / `AllColumn` in the Tier 1 rules parser, and through a
  lenient `filter scope` classification (`one column` / `any column` /
  `all columns`, each with a description) in the GLiNER backend. A
  negated `no column is empty` / `has no empty column` flips to
  `all notempty`; a missing scope label stays single-column.
- [ ] Verify the real model, validation, and UI regression tests.
- [ ] Compound model extraction with AND/OR grouping (follow-up; existing
  deterministic compound parsing remains available).
- [ ] Provide model download progress and cancellation for distribution.

### Dependencies

- Embedded generative backend: optional Candle runtime (`nl-model`).
- Native GLiNER backend (`nl-gliner`, off by default): pinned `gliner-rs`
  0.2.3 uses Candle 0.11, matching the existing inference runtime. It also
  brings tokenizers 0.22 and CLI dependencies. See ADR 0002 for the
  maintenance tradeoff and explicit model setup command.

### Acceptance criteria

- `Cmd/Ctrl+Shift+L` opens the Ask bar; a clean phrase such as
  "country is US and amount over 100" previews
  `country == US AND amount > 100`, and accepting it runs the existing
  filter path. (Value-first/implied-column phrasing such as
  "US orders over 100" is intentionally not clean, so it escalates to
  Tier 2 or prompts the user.)
- An unparseable phrase leaves the filter unchanged and shows what could
  not be understood.
- The default build has no new dependency and makes no network calls.
- `cargo test --release` passes; the rules parser has a phrase-table test
  suite.

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

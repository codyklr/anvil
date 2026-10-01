//! The Anvil main view: toolbar, virtualized grid, status bar, archive picker.
//!
//! Threading model: opening/parsing/indexing runs on GPUI's background
//! executor. Each open bumps a generation counter; results from a stale
//! generation are dropped, which makes "drop a new file while the old one
//! is still loading" safe.
//!
//! Submodules hold the search bar (`search`), the filter bar (`filter`),
//! and the `#[cfg(test)]` UI suite (`tests`). The shared editable-text
//! rendering lives in `super::input`; the edit buffer itself is
//! `super::text_edit`.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    canvas, div, prelude::*, px, rgb, uniform_list, AnyElement, Bounds, Context, ExternalPaths,
    FocusHandle, KeyDownEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render,
    ScrollDelta, ScrollHandle, ScrollStrategy, ScrollWheelEvent, Task, UniformListScrollHandle,
    Window,
};

use anvil_core::{
    apply_filter_expr, column_stats, describe_nl, find_ranges, infer_schema, parse_filter_expr,
    parse_nl, reindex, sample_distinct, scan_encoding, search_records, sort_rows,
    suggest_candidates, suggest_context, ArchiveSource, CellPos, ColumnIndex, ColumnStats,
    DistinctCount, EncodingReport, FilterExpr, HeaderMode, IndexOptions, LoadPhase, LoadProgress,
    LoadResult, LoadedDocument, NlColumn, NlResult, NlSchema, ParsedDocument, SearchOptions,
    Selection, Slot, SortKind, SortOrder, SortSpec, StatsOptions,
};

use super::archive_picker;
use super::commands::{self, Action};
use super::grid::{self, GridModel};
use super::input;
use super::text_edit::TextEdit;
use super::theme;

mod ask;
mod diagnostics;
mod filter;
mod goto;
mod search;

/// How long the green "copied" flash stays over the selection before it
/// fades back to the normal selection colour.
const COPY_FLASH_DURATION: Duration = Duration::from_millis(450);

/// How often the UI samples the shared progress tracker while a load is
/// running. Fast enough to feel live, slow enough to be free.
const PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How often the drag auto-scroll task advances the view while the
/// pointer is held outside the grid's top or bottom edge.
const AUTO_SCROLL_TICK: Duration = Duration::from_millis(30);

/// Writes text to the system clipboard. Stored on `MainView` so UI
/// tests can substitute a recorder and assert the exact TSV without
/// clobbering the real clipboard.
#[allow(clippy::type_complexity)]
type ClipboardWriter = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Shared, wait-free progress state for one background open. The data
/// task stores samples through a [`ProgressSink`]; the UI polls this on
/// a short timer. Atomics keep it `Send + Sync` and free of any GPUI
/// dependency, so the background executor can write it directly.
#[derive(Default)]
struct ProgressTracker {
    phase: AtomicU8,
    done: AtomicU64,
    /// `0` means "total unknown" (indeterminate).
    total: AtomicU64,
    finished: AtomicBool,
}

impl ProgressTracker {
    fn store(&self, progress: LoadProgress) {
        self.phase.store(progress.phase.as_u8(), Ordering::Relaxed);
        self.done.store(progress.bytes_done, Ordering::Relaxed);
        self.total
            .store(progress.bytes_total.unwrap_or(0), Ordering::Relaxed);
    }

    fn snapshot(&self) -> LoadProgress {
        let total = self.total.load(Ordering::Relaxed);
        LoadProgress {
            phase: LoadPhase::from_u8(self.phase.load(Ordering::Relaxed)),
            bytes_done: self.done.load(Ordering::Relaxed),
            bytes_total: (total > 0).then_some(total),
        }
    }

    fn mark_finished(&self) {
        self.finished.store(true, Ordering::Relaxed);
    }

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
}

/// Per-document state captured when a tab is no longer active and
/// restored when the user switches back to it. The fields mirror
/// the corresponding live fields on `MainView` so render and test
/// code can keep reading from one place.
#[derive(Clone)]
struct DocumentSnapshot {
    /// Stable identifier assigned when the tab is created. Lazy-load
    /// completion looks the tab up by id, so a tab closed mid-load
    /// cannot be resurrected and a reordered list cannot confuse the
    /// result with a different tab.
    id: u64,
    /// Arc-shared parsed document, or `None` while a ZIP-backed tab
    /// is waiting to be extracted and indexed. Tabs for every usable
    /// archive entry are created immediately when the ZIP is opened;
    /// only the active one is loaded on demand.
    doc: Option<Arc<ParsedDocument>>,
    /// Set when this document came from a ZIP entry; the picker
    /// overlay consults it and lazy loads use it to locate the entry.
    archive: Option<Arc<ArchiveSource>>,
    /// The archive entry this document was loaded from.
    entry_name: Option<String>,
    /// True while a lazy load for this tab is in flight, so repeated
    /// `switch_to` calls do not spawn duplicate extraction tasks.
    loading: bool,
    /// Cell / range selection.
    selection: Selection,
    /// Horizontal scroll position in pixels.
    scroll_x: f32,
    /// Vertical scroll handle. Each snapshot owns its own so tabs
    /// scroll independently. The live `MainView.v_scroll` is a clone
    /// of the active snapshot's handle.
    v_scroll: UniformListScrollHandle,
    /// Whether the search bar is visible for this tab.
    search_open: bool,
    /// The needle typed into the search bar.
    search_query: TextEdit,
    /// Sorted record indexes that match the latest query. Reset on
    /// every keystroke while a new search is in flight.
    search_matches: Vec<u64>,
    /// Index into `search_matches` of the active match, or `-1`.
    search_active: i64,
    /// Bumped on every keystroke; obsolete search tasks compare
    /// their captured generation against this and discard themselves.
    search_generation: u64,
    /// Whether this tab's search is byte-exact (`Aa` toggle in the
    /// search bar). Per tab so a case-sensitive hunt in one file does
    /// not surprise the user in another.
    search_case_sensitive: bool,
    /// Restrict this tab's search (and match highlighting) to a single
    /// column, set by clicking a column header. `None` searches every
    /// column. Persisted across closing the bar so the
    /// "click a header, then Cmd/Ctrl+F" flow works; the scoped
    /// header stays highlighted so the state is visible.
    search_column: Option<usize>,
    /// Rectangle of the most recent successful copy, painted green
    /// until the flash timer clears it.
    copy_flash_rect: Option<(CellPos, CellPos)>,
    copy_flash_generation: u64,
    /// Text in the filter input (kept per tab so switching away and
    /// back restores what the user was typing).
    filter_input: TextEdit,
    /// The applied predicate expression, or `None` when the tab is
    /// unfiltered.
    filter_applied: Option<FilterExpr>,
    /// Record indexes that pass the filter, in record order. `None`
    /// means the tab is unfiltered and the grid renders the whole
    /// document.
    filter_rows: Option<Arc<Vec<u64>>>,
    /// Bumped on every filter apply; a background filter task whose
    /// generation has moved on discards its result.
    filter_generation: u64,
    /// Active sort for this tab, or `None`. `kind` is filled in from
    /// inferred column types when the sort task runs; the stored value
    /// only needs the column and direction for cycling.
    sort_spec: Option<SortSpec>,
    /// Record indexes in sorted display order, a permutation of the
    /// current base (filtered set, or every record). `None` while no
    /// sort is active or while the task is in flight.
    sort_rows: Option<Arc<Vec<u64>>>,
    /// Bumped on every sort request; stale background results are
    /// discarded.
    sort_generation: u64,
    /// How this tab interprets its first row. `Auto` is the detected
    /// heuristic; `Header` / `Headerless` are explicit user overrides
    /// applied by re-indexing the same opened source.
    header_mode: HeaderMode,
    /// Per-column widths after a manual header resize, or `None` for the
    /// automatic widths. Restored when the tab becomes active so widths
    /// survive tab switches.
    column_widths: Option<Vec<f32>>,
}

impl DocumentSnapshot {
    /// A fully-loaded tab (direct file, or a ZIP entry that has
    /// already been extracted and indexed).
    fn from_doc(
        id: u64,
        doc: Arc<ParsedDocument>,
        archive: Option<Arc<ArchiveSource>>,
        entry_name: Option<String>,
    ) -> Self {
        Self {
            id,
            doc: Some(doc),
            archive,
            entry_name,
            loading: false,
            selection: Selection::empty(),
            scroll_x: 0.0,
            v_scroll: UniformListScrollHandle::new(),
            search_open: false,
            search_query: TextEdit::new(),
            search_matches: Vec::new(),
            search_active: -1,
            search_generation: 0,
            copy_flash_rect: None,
            copy_flash_generation: 0,
            filter_input: TextEdit::new(),
            filter_applied: None,
            filter_rows: None,
            filter_generation: 0,
            sort_spec: None,
            sort_rows: None,
            sort_generation: 0,
            search_case_sensitive: false,
            search_column: None,
            header_mode: HeaderMode::Auto,
            column_widths: None,
        }
    }

    /// A ZIP entry tab that has not been loaded yet. Created for
    /// every usable archive entry when the archive is opened; the
    /// document is extracted and indexed the first time the tab
    /// becomes active.
    fn pending_entry(id: u64, archive: Arc<ArchiveSource>, entry_name: String) -> Self {
        Self {
            id,
            doc: None,
            archive: Some(archive),
            entry_name: Some(entry_name),
            loading: false,
            selection: Selection::empty(),
            scroll_x: 0.0,
            v_scroll: UniformListScrollHandle::new(),
            search_open: false,
            search_query: TextEdit::new(),
            search_matches: Vec::new(),
            search_active: -1,
            search_generation: 0,
            copy_flash_rect: None,
            copy_flash_generation: 0,
            filter_input: TextEdit::new(),
            filter_applied: None,
            filter_rows: None,
            filter_generation: 0,
            sort_spec: None,
            sort_rows: None,
            sort_generation: 0,
            search_case_sensitive: false,
            search_column: None,
            header_mode: HeaderMode::Auto,
            column_widths: None,
        }
    }

    fn is_loaded(&self) -> bool {
        self.doc.is_some()
    }

    /// Short label for the tab strip. Truncated to a sane length.
    fn tab_label(&self) -> String {
        let raw = match (&self.archive, &self.entry_name) {
            // ZIP-backed tab: the entry name on its own is usually
            // enough; the archive name is only shown when two entries
            // in different archives could collide. We keep it short
            // for the strip by showing just the entry basename.
            (Some(_), Some(e)) => e.rsplit(['/', '\\']).next().unwrap_or(e).to_string(),
            _ => match &self.doc {
                Some(doc) => doc.opened.display_label(),
                None => "loading…".to_string(),
            },
        };
        if raw.chars().count() > 40 {
            let truncated: String = raw.chars().take(37).collect();
            format!("{truncated}…")
        } else {
            raw
        }
    }
}

/// A group of documents that share an archive (or are all direct
/// files). `archive` is `None` for direct files; `entries` is a list
/// of `(tab_index, label, is_loaded)` in tab order.
type FileGroup = (Option<Arc<ArchiveSource>>, Vec<(usize, String, bool)>);

/// Group consecutive snapshots that come from the same archive (or
/// are all direct files) for the file-manager panel. The list is in
/// tab order, so grouping consecutive runs preserves the order the
/// user opened things in. Two different archives with the same file
/// name stay in separate groups because grouping is by `Arc`
/// identity, not by name.
fn group_documents(documents: &[DocumentSnapshot]) -> Vec<FileGroup> {
    let mut groups: Vec<FileGroup> = Vec::new();
    for (index, snap) in documents.iter().enumerate() {
        let key = snap.archive.clone();
        let label = snap.tab_label();
        let loaded = snap.is_loaded();
        if let Some(last) = groups.last_mut() {
            let same = match (&last.0, &key) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            };
            if same {
                last.1.push((index, label, loaded));
                continue;
            }
        }
        groups.push((key, vec![(index, label, loaded)]));
    }
    groups
}

/// Default `ClipboardWriter`: `arboard` (text only). Errors are turned
/// into strings so the status bar can display them.
fn system_clipboard_write(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text.to_owned()))
        .map_err(|err| err.to_string())
}

/// Result of the background open+index pass.
enum OpenOutcome {
    Ready(Box<LoadedDocument>),
    NeedArchiveChoice(Arc<ArchiveSource>),
    Error(String),
}

#[derive(Default)]
enum ViewState {
    #[default]
    Empty,
    Loading(String),
    Ready,
    Error(String),
}

pub struct MainView {
    /// Multi-document state. Every successfully opened file (and
    /// every chosen ZIP entry) gets its own `DocumentSnapshot`. The
    /// active tab's snapshot is mirrored into the "live" per-document
    /// fields below so render and test code can keep reading them
    /// directly.
    documents: Vec<DocumentSnapshot>,
    /// Index into `documents` of the active tab, or `None` when no
    /// document is open. Switching tabs snapshots the current live
    /// state into `documents[active]` and loads the new snapshot's
    /// state into the live fields.
    active: Option<usize>,
    /// Monotonic counter for `DocumentSnapshot::id`. Bumped on every
    /// tab creation; never reused.
    next_doc_id: u64,
    /// Whether the file-manager side panel is visible. Toggled with
    /// Cmd/Ctrl+B and the toolbar button; defaults to open.
    file_panel_open: bool,
    /// Horizontal scroll handle for the tab strip. With many tabs the
    /// strip overflows the window; this lets it scroll instead of
    /// crushing every tab.
    tab_scroll: ScrollHandle,

    generation: u64,
    model: Option<GridModel>,
    state: ViewState,
    archive: Option<Arc<ArchiveSource>>,
    selected_entry: Option<String>,
    show_picker: bool,

    scroll_x: f32,
    viewport_w: f32,
    v_scroll: UniformListScrollHandle,
    /// `(mouse_x_at_drag_start, scroll_x_at_drag_start)` while the user
    /// drags the horizontal scrollbar thumb.
    h_drag: Option<(f32, f32)>,
    /// Active column-resize drag: `(column, pointer x at press, width
    /// at press)`. Set by a header boundary handle; a global mouse-move
    /// updates the width and mouse-up ends the drag.
    column_drag: Option<(usize, f32, f32)>,

    selection: Selection,
    /// How the active document's first row is interpreted. Mirrors
    /// `documents[active].header_mode`. (Mirror.)
    header_mode: HeaderMode,
    /// True between a mouse-down on a grid cell and the matching
    /// mouse-up. Drag-select only extends the selection while this is
    /// set, so a drag that began outside the grid does not suddenly
    /// select cells when it passes over them.
    drag_selecting: bool,

    task: Option<Task<()>>,
    /// Transient status-bar message (e.g. "copied 12 cells"). Cleared
    /// when the user makes another selection-changing action.
    status_message: Option<String>,

    /// Focus handle for the scrollable grid. Keyboard events (arrow
    /// navigation, Cmd/Ctrl+C) are only delivered to `on_grid_key_down`
    /// while this handle is focused, so the grid carries
    /// `.track_focus(&self.focus_handle)` and the view focuses it when
    /// a document is applied (and on any mouse-down inside the grid).
    focus_handle: FocusHandle,
    /// Set when a document becomes ready; the next render schedules a
    /// deferred `window.focus`, which is the safe point to move focus
    /// (mutating focus mid-render can re-enter the frame).
    focus_grid_on_next_frame: bool,

    /// Clipboard sink. Defaults to `arboard`; tests inject a recorder.
    clipboard: ClipboardWriter,

    /// Normalized rectangle of the most recent successful copy, painted
    /// with `theme::BG_COPIED` until the flash timer clears it. `None`
    /// when no flash is active.
    copy_flash_rect: Option<(CellPos, CellPos)>,
    /// Bumped on every copy. A flash timer only clears the rectangle if
    /// its captured generation is still current, so copying again while
    /// a flash is active restarts the flash instead of being cleared
    /// early by the previous timer.
    copy_flash_generation: u64,

    /// Window-space bounds of the scrollable grid area, recorded by the
    /// overlay canvas each frame. Two consumers: it decides when a drag
    /// has left the list vertically and should auto-scroll, and its
    /// width (minus the row-number gutter) is the measured
    /// `viewport_w` used by the horizontal scroll math.
    grid_bounds: Option<Bounds<Pixels>>,
    /// Last observed pointer position in window coordinates, updated by
    /// the overlay canvas's global mouse-move listener. Reported even
    /// when the pointer is outside the grid, which per-cell handlers
    /// cannot see.
    pointer_pos: Option<Point<Pixels>>,
    /// Ticks while a drag is active and the pointer sits outside the
    /// list, advancing the view and growing the selection. Dropped on
    /// mouse-up (or cleared by the loop itself when the drag ends).
    auto_scroll_task: Option<Task<()>>,

    /// Whether the right-side column-inspector panel is visible. Toggled
    /// by Cmd/Ctrl+I and the toolbar button. Global; it follows the
    /// active tab's selected cell.
    inspector_open: bool,
    /// The column whose stats are currently shown, or `None`.
    inspector_column: Option<usize>,
    /// Stats for `inspector_column`, or `None` while a scan is in flight.
    inspector_stats: Option<Arc<ColumnStats>>,
    /// Bumped on every inspector request; stale results are discarded.
    inspector_generation: u64,
    /// Outstanding background column-stats task.
    inspector_task: Option<Task<()>>,

    /// Whether the right-side diagnostics panel is visible. Global,
    /// like the inspector and file panels. The warnings themselves are
    /// immutable per document and read live from the active model, so
    /// no background task or generation counter is needed.
    diagnostics_open: bool,
    /// UTF-8 scan of the active document, computed on the background
    /// executor when the diagnostics panel opens. `None` while the scan
    /// is in flight (or before the panel is opened). Unlike warnings,
    /// this is O(file) so it cannot run during render.
    encoding_report: Option<Arc<EncodingReport>>,
    /// Discards obsolete encoding scans after a document change.
    encoding_generation: u64,
    /// Outstanding background encoding-scan task.
    encoding_task: Option<Task<()>>,

    /// Search state (Cmd/Ctrl+F). `search_open` controls whether the
    /// search bar is visible. While open, `search_query` holds the
    /// user's typed needle; each keystroke spawns a background
    /// `query::search` task with `search_generation` so obsolete work
    /// is dropped. `search_matches` is the most recent result; it can
    /// be briefly stale relative to `search_query` while a search is
    /// in flight.
    search_open: bool,
    search_query: TextEdit,
    search_matches: Vec<u64>,
    /// Index into `search_matches` of the currently highlighted row.
    /// `-1` when there are no matches yet. Cycled by Enter / Shift+Enter
    /// inside the bar and by F3 / Shift-F3 anywhere.
    search_active: i64,
    /// Bumped on every keystroke / open / close. A background task
    /// only applies its result if the view's generation still matches.
    search_generation: u64,
    /// Whether search is byte-exact for the active tab. (Mirror.)
    search_case_sensitive: bool,
    /// Restrict search to one column index, or `None` for all
    /// columns. (Mirror.)
    search_column: Option<usize>,
    /// Outstanding search task. Dropped when the view closes the bar
    /// or starts a new search; the running task continues but its
    /// `update` callback becomes a no-op.
    search_task: Option<Task<()>>,
    /// Focus handle for the search bar overlay. Independent of the grid
    /// handle so the search bar can swallow keystrokes (typed
    /// characters, Enter to advance) without the grid's `MoveCursor`
    /// arm firing.
    search_focus_handle: FocusHandle,
    /// Set when `open_search` is called; the next render schedules a
    /// deferred `window.focus` onto `search_focus_handle`. Mirrors the
    /// pattern used for the grid handle.
    focus_search_on_next_frame: bool,
    /// Window-space bounds of the search input's text area, recorded
    /// each frame by a `canvas` overlay; maps pointer x to a byte index.
    search_text_bounds: Option<Bounds<Pixels>>,
    /// True between a mouse-down in the search input and the matching
    /// mouse-up; drag-select only extends while set.
    search_dragging: bool,

    /// Whether the filter bar (Cmd/Ctrl+L) is visible. The bar's
    /// visibility is global; the predicate itself is per tab.
    filter_open: bool,
    /// Text in the filter input. (Mirror of `documents[active]`.)
    filter_input: TextEdit,
    /// The applied predicate for the active tab, or `None`.
    /// (Mirror.)
    filter_applied: Option<FilterExpr>,
    /// Record indexes that pass the active tab's filter, or `None` when
    /// the tab is unfiltered. (Mirror.)
    filter_rows: Option<Arc<Vec<u64>>>,
    /// Bumped on every apply; obsolete filter tasks discard their
    /// results. (Mirror.)
    filter_generation: u64,
    /// Parse or evaluation error from the last apply, shown in the bar.
    filter_error: Option<String>,
    /// Outstanding background filter task.
    filter_task: Option<Task<()>>,
    /// Active sort for the active tab, or `None`. (Mirror.)
    sort_spec: Option<SortSpec>,
    /// Sorted record indexes for the active tab, or `None`. (Mirror.)
    sort_rows: Option<Arc<Vec<u64>>>,
    /// Bumped on every sort request; obsolete sort tasks discard their
    /// results. (Mirror.)
    sort_generation: u64,
    /// Outstanding background sort task.
    sort_task: Option<Task<()>>,
    /// Focus handle for the filter bar. Kept separate from the search
    /// and grid handles so each bar's keys route correctly.
    filter_focus_handle: FocusHandle,
    /// Set when `open_filter` is called; the next render schedules a
    /// deferred focus onto `filter_focus_handle`.
    focus_filter_on_next_frame: bool,
    /// Window-space bounds of the filter input's text area, recorded
    /// each frame by a `canvas` overlay. Pointer x is mapped through
    /// this to a byte index for click/drag caret placement.
    filter_text_bounds: Option<Bounds<Pixels>>,
    /// True between a mouse-down in the filter input and the matching
    /// mouse-up; drag-select only extends while set.
    filter_dragging: bool,
    /// Autocomplete rows for the filter input, recomputed after every
    /// edit/caret move. Empty when there is nothing to suggest.
    filter_suggestions: Vec<FilterSuggestion>,
    /// Highlighted row in `filter_suggestions`.
    filter_suggestion_selected: usize,
    /// Whether the autocomplete popup is currently shown. Closed by Esc,
    /// accepting a suggestion, or clearing the input.
    filter_popup_open: bool,
    /// Bounded distinct-value samples per column, used for the value
    /// slot. Cleared when the active document changes.
    filter_value_cache: HashMap<usize, Vec<String>>,
    /// Scroll handle for the autocomplete popup, so the highlighted row
    /// is kept in view as the selection moves.
    filter_suggestion_scroll: ScrollHandle,
    /// Whether the natural-language Ask bar (Cmd/Ctrl+Shift+L) is open.
    ask_open: bool,
    /// Text in the Ask input.
    ask_input: TextEdit,
    /// Latest parse of `ask_input`, shown as a preview before accepting.
    ask_result: Option<NlResult>,
    /// Schema context for the active document, built when the bar opens.
    ask_schema: Option<NlSchema>,
    ask_focus_handle: FocusHandle,
    /// Set when `open_ask` is called; the next render focuses the input.
    focus_ask_on_next_frame: bool,
    ask_text_bounds: Option<Bounds<Pixels>>,
    ask_dragging: bool,
    /// True while a Tier 2 interpretation is running on the background
    /// executor.
    ask_loading: bool,
    /// Error from the last Tier 2 attempt, shown in the bar.
    ask_model_error: Option<String>,
    /// Discards stale Tier 2 results when the input changes.
    ask_generation: u64,
    ask_task: Option<Task<()>>,
    /// Optional Tier 2 interpreter (embedded model when the `nl-model`
    /// feature is on; injectable in tests).
    ask_interpreter: Option<AskInterpreter>,

    /// Whether the go-to-row input bar (Cmd/Ctrl+G) is visible.
    goto_open: bool,
    /// Text + caret for the go-to-row input.
    goto_input: TextEdit,
    /// Inline error from the last Enter, e.g. a non-numeric entry.
    goto_error: Option<String>,
    /// Focus handle for the go-to-row bar, kept separate from the other
    /// bars so its keys route to `on_goto_key_down`.
    goto_focus_handle: FocusHandle,
    /// Set when the bar opens; the next render schedules a deferred
    /// focus onto `goto_focus_handle`.
    focus_goto_on_next_frame: bool,
    /// Window-space bounds of the go-to-row input, for pointer
    /// hit-testing.
    goto_text_bounds: Option<Bounds<Pixels>>,
    /// True between a mouse-down and mouse-up in the go-to-row input.
    goto_dragging: bool,

    /// Shared progress state for the background open currently tracked,
    /// or `None` when nothing is loading. `progress_snapshot` is the
    /// polled value the renderer draws.
    loading_progress: Option<Arc<ProgressTracker>>,
    progress_snapshot: Option<LoadProgress>,
    /// Epoch of the tracked load. A poller stops when a newer load has
    /// bumped it, which discards obsolete updates.
    progress_generation: u64,
    /// Outstanding progress poller.
    progress_task: Option<Task<()>>,
}

/// A Tier 2 interpreter: phrase + schema + value source to an expression.
/// The result is always validated by `anvil_core::validate_nl_candidate`
/// inside the interpreter, so it cannot become an arbitrary query.
pub(crate) type AskInterpreter = Arc<
    dyn Fn(&str, &NlSchema, &dyn Fn(usize) -> Vec<String>) -> Result<FilterExpr, String>
        + Send
        + Sync,
>;

/// The production Tier 2 interpreter. With the `nl-model` feature this is
/// the embedded model backend; without it, Tier 2 is unavailable and the
/// Ask bar only uses the deterministic parser.
#[cfg(feature = "nl-gliner")]
fn ask_interpreter_default() -> Option<AskInterpreter> {
    let interpreter = anvil_core::gliner::Interpreter::default();
    Some(Arc::new(move |phrase, schema, values| {
        interpreter.interpret(phrase, schema, values)
    }))
}

#[cfg(all(feature = "nl-model", not(feature = "nl-gliner")))]
fn ask_interpreter_default() -> Option<AskInterpreter> {
    Some(Arc::new(|phrase, schema, values| {
        crate::nlmodel::interpret(phrase, schema, values)
    }))
}

#[cfg(not(any(feature = "nl-model", feature = "nl-gliner")))]
fn ask_interpreter_default() -> Option<AskInterpreter> {
    None
}

/// One row in the filter autocomplete popup: the text to insert and the
/// byte range of the input it replaces. `quote` wraps the inserted text
/// in double quotes, needed for column names that contain whitespace so
/// they stay a single token.
#[derive(Debug, Clone)]
struct FilterSuggestion {
    text: String,
    replace: Range<usize>,
    quote: bool,
}

impl MainView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            documents: Vec::new(),
            active: None,
            next_doc_id: 0,
            file_panel_open: true,
            tab_scroll: ScrollHandle::new(),
            generation: 0,
            model: None,
            state: ViewState::Empty,
            archive: None,
            selected_entry: None,
            show_picker: false,
            scroll_x: 0.0,
            viewport_w: 1600.0,
            v_scroll: UniformListScrollHandle::new(),
            h_drag: None,
            column_drag: None,
            selection: Selection::empty(),
            header_mode: HeaderMode::Auto,
            drag_selecting: false,
            task: None,
            status_message: None,
            focus_handle: cx.focus_handle(),
            focus_grid_on_next_frame: false,
            clipboard: Arc::new(system_clipboard_write),
            copy_flash_rect: None,
            copy_flash_generation: 0,
            grid_bounds: None,
            pointer_pos: None,
            auto_scroll_task: None,
            inspector_open: false,
            inspector_column: None,
            inspector_stats: None,
            inspector_generation: 0,
            inspector_task: None,
            diagnostics_open: false,
            encoding_report: None,
            encoding_generation: 0,
            encoding_task: None,
            search_open: false,
            search_query: TextEdit::new(),
            search_matches: Vec::new(),
            search_active: -1,
            search_generation: 0,
            search_case_sensitive: false,
            search_column: None,
            search_task: None,
            search_focus_handle: cx.focus_handle(),
            focus_search_on_next_frame: false,
            search_text_bounds: None,
            search_dragging: false,
            filter_open: false,
            filter_input: TextEdit::new(),
            filter_applied: None,
            filter_rows: None,
            filter_generation: 0,
            filter_error: None,
            filter_task: None,
            sort_spec: None,
            sort_rows: None,
            sort_generation: 0,
            sort_task: None,
            filter_focus_handle: cx.focus_handle(),
            focus_filter_on_next_frame: false,
            filter_text_bounds: None,
            filter_dragging: false,
            filter_suggestions: Vec::new(),
            filter_suggestion_selected: 0,
            filter_popup_open: false,
            filter_value_cache: HashMap::new(),
            filter_suggestion_scroll: ScrollHandle::new(),
            ask_open: false,
            ask_input: TextEdit::new(),
            ask_result: None,
            ask_schema: None,
            ask_focus_handle: cx.focus_handle(),
            focus_ask_on_next_frame: false,
            ask_text_bounds: None,
            ask_dragging: false,
            ask_loading: false,
            ask_model_error: None,
            ask_generation: 0,
            ask_task: None,
            ask_interpreter: ask_interpreter_default(),
            goto_open: false,
            goto_input: TextEdit::new(),
            goto_error: None,
            goto_focus_handle: cx.focus_handle(),
            focus_goto_on_next_frame: false,
            goto_text_bounds: None,
            goto_dragging: false,
            loading_progress: None,
            progress_snapshot: None,
            progress_generation: 0,
            progress_task: None,
        }
    }

    // ---- multi-document / tabs -----------------------------------------

    /// Copy the current live per-document state into the snapshot at
    /// `active`. Called before mutating live state (e.g. when a new
    /// document is loaded) or before switching tabs so the snapshot
    /// reflects the user's latest actions.
    fn snapshot_active(&mut self) {
        let Some(active) = self.active else { return };
        let Some(snap) = self.documents.get_mut(active) else {
            return;
        };
        snap.selection = self.selection.clone();
        snap.scroll_x = self.scroll_x;
        snap.v_scroll = self.v_scroll.clone();
        snap.search_open = self.search_open;
        snap.search_query = self.search_query.clone();
        snap.search_matches = self.search_matches.clone();
        snap.search_active = self.search_active;
        snap.search_generation = self.search_generation;
        snap.search_case_sensitive = self.search_case_sensitive;
        snap.search_column = self.search_column;
        snap.copy_flash_rect = self.copy_flash_rect;
        snap.copy_flash_generation = self.copy_flash_generation;
        snap.filter_input = self.filter_input.clone();
        snap.filter_applied = self.filter_applied.clone();
        snap.filter_rows = self.filter_rows.clone();
        snap.filter_generation = self.filter_generation;
        snap.sort_spec = self.sort_spec;
        snap.sort_rows = self.sort_rows.clone();
        snap.sort_generation = self.sort_generation;
        snap.header_mode = self.header_mode;
        snap.archive = self.archive.clone();
        snap.entry_name = self.selected_entry.clone();
        snap.column_widths = self.model.as_ref().map(|model| model.layout.widths.clone());
    }

    /// Reset all live per-document fields to the "no document"
    /// defaults. Used when there is no active document (empty
    /// state) so render code observes a consistent view.
    fn clear_live_per_document(&mut self) {
        self.model = None;
        self.archive = None;
        self.selected_entry = None;
        self.selection = Selection::empty();
        self.scroll_x = 0.0;
        self.v_scroll = UniformListScrollHandle::new();
        self.search_open = false;
        self.search_query.clear();
        self.search_matches.clear();
        self.search_active = -1;
        self.search_generation = 0;
        self.search_case_sensitive = false;
        self.search_column = None;
        self.copy_flash_rect = None;
        self.copy_flash_generation = 0;
        self.filter_input.clear();
        self.filter_applied = None;
        self.filter_rows = None;
        self.filter_generation = 0;
        self.sort_spec = None;
        self.sort_rows = None;
        self.sort_generation = 0;
        self.header_mode = HeaderMode::Auto;
    }

    /// Copy `documents[new_active]` into the live per-document fields.
    /// Called after switching tabs so the next render reflects the
    /// new tab.
    fn load_active(&mut self) {
        // The document may differ now; drop any inspector stats.
        self.invalidate_inspector();
        self.invalidate_encoding();
        // Column value samples belong to the previous document.
        self.filter_value_cache.clear();
        // The Ask schema/preview belong to the previous document.
        self.invalidate_ask();
        let Some(active) = self.active else {
            self.clear_live_per_document();
            return;
        };
        let Some(snap) = self.documents.get(active).cloned() else {
            self.clear_live_per_document();
            return;
        };
        // A pending (not-yet-loaded) tab has no document yet; render
        // code shows a loading placeholder for it until the lazy load
        // completes.
        self.model = snap.doc.clone().map(GridModel::new);
        // Restore manually-resized column widths for this tab. The
        // length check guards a reindex that changed the column count.
        if let (Some(model), Some(widths)) = (self.model.as_mut(), snap.column_widths.as_ref()) {
            if model.layout.widths.len() == widths.len() {
                model.layout = Arc::new(grid::ColumnLayout::from_widths(widths.clone()));
            }
        }
        self.archive = snap.archive;
        self.selected_entry = snap.entry_name;
        self.selection = snap.selection;
        self.scroll_x = snap.scroll_x;
        self.v_scroll = snap.v_scroll;
        self.search_open = snap.search_open;
        self.search_query = snap.search_query;
        self.search_matches = snap.search_matches;
        self.search_active = snap.search_active;
        self.search_generation = snap.search_generation;
        self.search_case_sensitive = snap.search_case_sensitive;
        self.search_column = snap.search_column;
        self.copy_flash_rect = snap.copy_flash_rect;
        self.copy_flash_generation = snap.copy_flash_generation;
        self.filter_input = snap.filter_input;
        self.filter_applied = snap.filter_applied;
        self.filter_rows = snap.filter_rows;
        self.filter_generation = snap.filter_generation;
        self.sort_spec = snap.sort_spec;
        self.sort_rows = snap.sort_rows;
        self.sort_generation = snap.sort_generation;
        self.header_mode = snap.header_mode;
    }

    /// Allocate the next unique `DocumentSnapshot` id.
    fn alloc_doc_id(&mut self) -> u64 {
        let id = self.next_doc_id;
        self.next_doc_id = self.next_doc_id.wrapping_add(1);
        id
    }

    /// Switch to the tab at `index`. No-op if it is already active
    /// or out of range.
    fn switch_to(&mut self, index: usize, cx: &mut Context<Self>) {
        if Some(index) == self.active {
            return;
        }
        if index >= self.documents.len() {
            return;
        }
        self.snapshot_active();
        self.active = Some(index);
        self.load_active();
        self.ensure_active_loaded(cx);
        self.kick_inspector(cx);
        self.kick_encoding(cx);
        // Focus the grid so the keyboard surface is ready. The same
        // deferred-focus path is used as after a fresh open.
        self.focus_grid_on_next_frame = true;
        cx.notify();
    }

    /// Close the tab at `index`. The active tab may shift to a
    /// neighbour (or `None` if this was the last tab).
    fn close_active_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.documents.len() {
            return;
        }
        let was_active = self.active == Some(index);
        self.documents.remove(index);
        let new_active = match self.active {
            None => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => {
                // Active tab was closed; pick a neighbour.
                if self.documents.is_empty() {
                    None
                } else {
                    Some(index.min(self.documents.len() - 1))
                }
            }
            Some(active) => Some(active),
        };
        self.active = new_active;
        self.load_active();
        if self.documents.is_empty() {
            // No tabs left - the centre panel falls back to the
            // empty state so the welcome screen reappears.
            self.state = ViewState::Empty;
        } else {
            // The neighbour may itself be a pending ZIP entry.
            self.ensure_active_loaded(cx);
        }
        if was_active && new_active.is_some() {
            self.focus_grid_on_next_frame = true;
        }
        cx.notify();
    }

    // ---- opening ---------------------------------------------------------

    /// Begin tracking progress for a new background load. Returns the
    /// shared tracker (the background task writes samples into it) and
    /// its epoch. An older poller stops on its next tick because its
    /// epoch no longer matches `progress_generation`, so obsolete
    /// updates are discarded.
    fn start_progress_tracking(&mut self, cx: &mut Context<Self>) -> (Arc<ProgressTracker>, u64) {
        self.progress_generation = self.progress_generation.wrapping_add(1);
        let generation = self.progress_generation;
        let tracker = Arc::new(ProgressTracker::default());
        self.loading_progress = Some(tracker.clone());
        self.progress_snapshot = Some(LoadProgress {
            phase: LoadPhase::Reading,
            bytes_done: 0,
            bytes_total: None,
        });

        let poller = tracker.clone();
        let task = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(PROGRESS_POLL_INTERVAL).await;
            let keep_going = this
                .update(cx, |view, cx| {
                    if view.progress_generation != generation {
                        return false;
                    }
                    if poller.is_finished() {
                        view.loading_progress = None;
                        view.progress_snapshot = None;
                        view.progress_task = None;
                        cx.notify();
                        return false;
                    }
                    view.refresh_progress();
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !keep_going {
                break;
            }
        });
        self.progress_task = Some(task);
        (tracker, generation)
    }

    /// Drop the tracked progress and stop its poller. Called when a load
    /// completes (success or error).
    fn clear_progress(&mut self, cx: &mut Context<Self>) {
        if let Some(tracker) = self.loading_progress.take() {
            tracker.mark_finished();
        }
        self.progress_snapshot = None;
        cx.notify();
    }

    /// Read the shared tracker into the render snapshot. Used by tests
    /// and by the poller.
    fn refresh_progress(&mut self) {
        self.progress_snapshot = self.loading_progress.as_ref().map(|t| t.snapshot());
    }

    /// Human-readable progress label, or `None` when nothing is loading.
    /// A known total shows bytes done over total; otherwise the phase is
    /// suffixed with an ellipsis (indeterminate).
    fn progress_text(&self) -> Option<String> {
        let progress = self.progress_snapshot?;
        let verb = match progress.phase {
            LoadPhase::Reading => "reading",
            LoadPhase::Extracting => "extracting",
            LoadPhase::Indexing => "indexing",
        };
        Some(match progress.bytes_total {
            Some(total) if total > 0 => format!(
                "{verb}  {} / {}",
                archive_picker::format_size(progress.bytes_done),
                archive_picker::format_size(total)
            ),
            _ => format!("{verb}…"),
        })
    }

    /// A small progress bar: determinate when the total is known, an
    /// indeterminate stub otherwise. `None` when nothing is loading.
    fn render_progress_bar(&self) -> Option<AnyElement> {
        const BAR_WIDTH: f32 = 280.0;
        const BAR_HEIGHT: f32 = 6.0;
        let progress = self.progress_snapshot?;
        let label = self.progress_text().unwrap_or_default();
        let mut bar = div()
            .w(px(BAR_WIDTH))
            .h(px(BAR_HEIGHT))
            .rounded_sm()
            .bg(theme::BORDER_STRONG);
        bar = match progress.fraction() {
            Some(fraction) => bar.child(
                div()
                    .w(px(BAR_WIDTH * fraction))
                    .h(px(BAR_HEIGHT))
                    .rounded_sm()
                    .bg(theme::ACCENT),
            ),
            None => bar.child(
                div()
                    .w(px(72.0))
                    .h(px(BAR_HEIGHT))
                    .rounded_sm()
                    .bg(theme::ACCENT),
            ),
        };
        Some(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(div().text_xs().text_color(theme::TEXT_DIM).child(label))
                .child(bar)
                .into_any_element(),
        )
    }

    fn begin_open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let (tracker, _) = self.start_progress_tracking(cx);
        self.state = ViewState::Loading(format!("reading {}", display_path(&path)));
        self.show_picker = false;
        cx.notify();

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let sink_tracker = tracker.clone();
            let outcome = executor
                .spawn(async move {
                    let sink = move |progress: LoadProgress| sink_tracker.store(progress);
                    load_and_index_with_progress(&path, &sink)
                })
                .await;
            tracker.mark_finished();
            this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.apply(outcome, cx);
            })
            .ok();
        });
        self.task = Some(task);
    }

    fn apply(&mut self, outcome: OpenOutcome, cx: &mut Context<Self>) {
        self.clear_progress(cx);
        match outcome {
            OpenOutcome::Ready(loaded) => {
                let LoadedDocument {
                    document,
                    archive,
                    entry,
                } = *loaded;
                let doc = Arc::new(document);
                match archive {
                    // A ZIP-backed open installs one tab per usable
                    // entry, with the loader's favourite pre-loaded
                    // and the rest waiting to be extracted on demand.
                    Some(source) => self.install_archive_tabs(source, entry, Some(doc), cx),
                    // A direct file is a single tab.
                    None => {
                        self.snapshot_active();
                        let id = self.alloc_doc_id();
                        self.documents
                            .push(DocumentSnapshot::from_doc(id, doc, None, None));
                        self.active = Some(self.documents.len() - 1);
                        self.load_active();
                        self.show_picker = false;
                        self.drag_selecting = false;
                        self.status_message = None;
                        self.focus_grid_on_next_frame = true;
                        self.state = ViewState::Ready;
                    }
                }
            }
            OpenOutcome::NeedArchiveChoice(source) => {
                // No clear favourite: every usable entry becomes a
                // pending tab and the first one loads immediately.
                self.install_archive_tabs(source, None, None, cx);
            }
            OpenOutcome::Error(message) => {
                // Surface the error in the status bar; do not tear
                // down the currently-open tab. (Before multi-document
                // support this arm replaced the single document with
                // an error state; in a multi-document world a
                // background-task error should not clobber the user's
                // other open files.)
                self.status_message = Some(format!("error: {message}"));
                // When no tab is open we still want to show the error
                // page (and not a stale "Loading..." from a previous
                // task). When at least one tab is open, the active
                // tab wins and the error lives only in the status
                // bar; the renderer enforces that.
                if self.documents.is_empty() {
                    self.state = ViewState::Error(message);
                }
            }
        }
        self.kick_inspector(cx);
        self.kick_encoding(cx);
        cx.notify();
    }

    /// Install one tab per usable archive entry. `loaded_entry` /
    /// `loaded_doc` are `Some` when the loader already produced a
    /// document for one of the entries (the auto-opened favourite);
    /// that entry becomes a ready tab and the others become pending
    /// tabs whose data is extracted on first activation.
    ///
    /// Every usable entry gets a tab immediately because the archive
    /// metadata (entry names and sizes) is available from the ZIP
    /// central directory without decompressing anything.
    fn install_archive_tabs(
        &mut self,
        source: Arc<ArchiveSource>,
        loaded_entry: Option<String>,
        loaded_doc: Option<Arc<ParsedDocument>>,
        cx: &mut Context<Self>,
    ) {
        self.snapshot_active();
        let usable: Vec<String> = source
            .candidates
            .iter()
            .filter(|c| c.usable)
            .map(|c| c.entry_name.clone())
            .collect();
        if usable.is_empty() {
            let message = "archive contains no usable data files".to_string();
            self.status_message = Some(format!("error: {message}"));
            if self.documents.is_empty() {
                self.state = ViewState::Error(message);
            }
            cx.notify();
            return;
        }

        let mut active_index = self.documents.len();
        for entry in usable {
            let id = self.alloc_doc_id();
            let is_loaded_favourite = loaded_entry.as_deref() == Some(entry.as_str());
            match (is_loaded_favourite, loaded_doc.as_ref()) {
                (true, Some(doc)) => {
                    self.documents.push(DocumentSnapshot::from_doc(
                        id,
                        doc.clone(),
                        Some(source.clone()),
                        Some(entry),
                    ));
                    active_index = self.documents.len() - 1;
                }
                _ => {
                    self.documents
                        .push(DocumentSnapshot::pending_entry(id, source.clone(), entry));
                }
            }
        }

        self.active = Some(active_index);
        self.load_active();
        self.ensure_active_loaded(cx);
        self.show_picker = false;
        self.drag_selecting = false;
        self.status_message = None;
        self.focus_grid_on_next_frame = true;
        self.state = ViewState::Ready;
        cx.notify();
    }

    /// Start a background extract+index pass for the active tab if it
    /// is a pending ZIP entry. No-op for loaded tabs and for tabs
    /// whose load is already in flight. The task is detached so
    /// switching away (or opening another file) does not cancel it
    /// and leave the tab permanently stuck in `loading`.
    fn ensure_active_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        let Some(snap) = self.documents.get(active) else {
            return;
        };
        if snap.is_loaded() || snap.loading {
            return;
        }
        let Some(archive) = snap.archive.clone() else {
            return;
        };
        let Some(entry) = snap.entry_name.clone() else {
            return;
        };
        let id = snap.id;
        if let Some(snap) = self.documents.get_mut(active) {
            snap.loading = true;
        }

        let executor = cx.background_executor().clone();
        let (tracker, _) = self.start_progress_tracking(cx);
        cx.spawn(async move |this, cx| {
            let sink_tracker = tracker.clone();
            let outcome = executor
                .spawn(async move {
                    let sink = move |progress: LoadProgress| sink_tracker.store(progress);
                    extract_and_index_with_progress(&archive, &entry, &sink)
                })
                .await;
            tracker.mark_finished();
            this.update(cx, |view, cx| view.apply_lazy_result(id, outcome, cx))
                .ok();
        })
        .detach();
    }

    /// Deliver the result of a lazy extraction to the tab it belongs
    /// to. The tab may have been closed while the extraction was in
    /// flight, in which case the result is discarded.
    fn apply_lazy_result(&mut self, id: u64, outcome: OpenOutcome, cx: &mut Context<Self>) {
        self.clear_progress(cx);
        let Some(index) = self.documents.iter().position(|d| d.id == id) else {
            return;
        };
        if let Some(snap) = self.documents.get_mut(index) {
            snap.loading = false;
        }
        match outcome {
            OpenOutcome::Ready(loaded) => {
                let LoadedDocument {
                    document,
                    archive,
                    entry,
                } = *loaded;
                if let Some(snap) = self.documents.get_mut(index) {
                    snap.doc = Some(Arc::new(document));
                    snap.archive = archive;
                    snap.entry_name = entry;
                }
                if self.active == Some(index) {
                    self.load_active();
                    self.focus_grid_on_next_frame = true;
                    self.kick_encoding(cx);
                }
                cx.notify();
            }
            OpenOutcome::Error(message) => {
                self.status_message = Some(format!("error: {message}"));
                // Remove the failed tab so the user is not left on a
                // loading placeholder. If it was active,
                // `close_active_at` picks a neighbour.
                self.close_active_at(index, cx);
            }
            OpenOutcome::NeedArchiveChoice(_) => {
                // Cannot happen for a single entry; ignore.
            }
        }
    }

    /// Switch to the tab backing `entry_name`, if one exists. Used by
    /// the archive picker when it lists a ZIP's contents.
    fn switch_to_entry(&mut self, entry_name: &str, cx: &mut Context<Self>) {
        let Some(index) = self
            .documents
            .iter()
            .position(|d| d.entry_name.as_deref() == Some(entry_name))
        else {
            return;
        };
        self.show_picker = false;
        self.switch_to(index, cx);
    }

    fn pick_file(&mut self, cx: &mut Context<Self>) {
        let picked = rfd::FileDialog::new()
            .set_title("Open a data file or archive")
            .add_filter(
                "Data files & archives",
                &["csv", "tsv", "tab", "txt", "psv", "dat", "zip"],
            )
            .pick_file();
        if let Some(path) = picked {
            self.begin_open(path, cx);
        }
    }

    // ---- rendering -------------------------------------------------------

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = match &self.model {
            Some(model) => model.doc.opened.display_label(),
            None => "No file open".to_string(),
        };
        let subtitle = self
            .archive
            .as_ref()
            .map(|a| format!("archive: {}", a.archive_name()));

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .h(theme::TOOLBAR_HEIGHT)
            .w_full()
            .px_3()
            .bg(theme::BG_TOOLBAR)
            .border_b_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .id("open-button")
                    .flex_shrink_0()
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .bg(theme::ACCENT)
                    .text_color(theme::TEXT_STRONG)
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0x1177bb)))
                    .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx)))
                    .child("Open…"),
            )
            // Toggle the file-manager panel. Disabled-looking while
            // there are no documents to list.
            .child(
                div()
                    .id("toggle-files")
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme::BORDER_STRONG)
                    .cursor_pointer()
                    .text_color(if self.file_panel_open {
                        theme::TEXT_STRONG
                    } else {
                        theme::TEXT_DIM
                    })
                    .hover(|s| s.bg(theme::BG_ROW_HOVER))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.file_panel_open = !this.file_panel_open;
                        cx.notify();
                    }))
                    .child("Files"),
            )
            // Toggle the column-inspector panel.
            .child(
                div()
                    .id("toggle-inspector")
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme::BORDER_STRONG)
                    .cursor_pointer()
                    .text_color(if self.inspector_open {
                        theme::TEXT_STRONG
                    } else {
                        theme::TEXT_DIM
                    })
                    .hover(|s| s.bg(theme::BG_ROW_HOVER))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_inspector(cx)))
                    .child("Inspector"),
            )
            // Toggle the diagnostics panel.
            .child(
                div()
                    .id("toggle-diagnostics")
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme::BORDER_STRONG)
                    .cursor_pointer()
                    .text_color(if self.diagnostics_open {
                        theme::TEXT_STRONG
                    } else {
                        theme::TEXT_DIM
                    })
                    .hover(|s| s.bg(theme::BG_ROW_HOVER))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_diagnostics(cx)))
                    .child("Diagnostics"),
            )
            // Cycle the first-row interpretation (Auto / Header / No header).
            .child(
                div()
                    .id("toggle-header")
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme::BORDER_STRONG)
                    .cursor_pointer()
                    .text_color(if self.header_mode == HeaderMode::Auto {
                        theme::TEXT_DIM
                    } else {
                        theme::TEXT_STRONG
                    })
                    .hover(|s| s.bg(theme::BG_ROW_HOVER))
                    .on_click(cx.listener(|this, _, _, cx| this.cycle_header_mode(cx)))
                    .child(header_mode_label(self.header_mode)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_hidden()
                    .child(div().text_color(theme::TEXT_STRONG).truncate().child(label))
                    .when_some(subtitle, |this, sub| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme::TEXT_DIM)
                                .truncate()
                                .child(sub),
                        )
                    }),
            )
            .when(self.archive.is_some(), |this| {
                this.child(
                    div()
                        .id("archive-browser")
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .border_1()
                        .border_color(theme::BORDER_STRONG)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::BG_ROW_HOVER))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_picker = true;
                            cx.notify();
                        }))
                        .child("Archive…"),
                )
            })
    }

    /// Horizontal strip of tabs above the grid. Each tab shows a
    /// short label for an open document. Click a tab to switch;
    /// click the × to close. When no documents are open, the bar
    /// is empty (no row rendered).
    ///
    /// Tabs live as a normal flex child of the root, below the
    /// toolbar and above the center area. They never overlap the
    /// grid the way the search bar used to.
    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.documents.is_empty() {
            return div().h(px(0.));
        }
        let mut tabs: Vec<AnyElement> = Vec::with_capacity(self.documents.len());
        for (index, snap) in self.documents.iter().enumerate() {
            let is_active = Some(index) == self.active;
            let label = snap.tab_label();
            let on_select_index = index;
            let on_close_index = index;

            let active_bg = theme::BG_PANEL;
            let inactive_bg = theme::BG_TOOLBAR;
            let active_fg = theme::TEXT_STRONG;
            let inactive_fg = theme::TEXT_DIM;
            let bg = if is_active { active_bg } else { inactive_bg };
            let fg = if is_active { active_fg } else { inactive_fg };

            // Tab body - clickable to switch active. `flex_shrink_0`
            // keeps tabs at their natural width; the strip scrolls
            // horizontally rather than crushing them.
            tabs.push(
                div()
                    .id(("doc-tab", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .h(px(f32::from(theme::TAB_BAR_HEIGHT)))
                    .px_2()
                    .bg(bg)
                    .border_r_1()
                    .border_color(theme::BORDER)
                    .cursor_pointer()
                    .hover(|s| if is_active { s } else { s.bg(theme::BG_PANEL) })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.switch_to(on_select_index, cx);
                    }))
                    .child(div().text_color(fg).child(label))
                    .child(
                        div()
                            .id(("doc-tab-close", index))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::TEXT_STRONG))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |view, _, _, cx| {
                                    view.close_active_at(on_close_index, cx);
                                }),
                            )
                            .child("×"),
                    )
                    .into_any_element(),
            );
        }

        // Outer strip: a full-width bar with a horizontally scrollable
        // inner track so a long tab list stays reachable.
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(f32::from(theme::TAB_BAR_HEIGHT)))
            .w_full()
            .bg(theme::BG_TOOLBAR)
            .border_b_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .id("tab-strip")
                    .flex()
                    .flex_row()
                    .items_center()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs),
            )
    }

    /// Left-hand file-manager panel. Lists every open document, with
    /// ZIP entries nested under their archive. Click a row to switch
    /// to it; click × to close it. Toggled with Cmd/Ctrl+B or the
    /// toolbar button; hidden by default when no documents are open.
    fn render_file_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = group_documents(&self.documents);
        let mut rows: Vec<AnyElement> = Vec::with_capacity(self.documents.len() + groups.len());
        for (archive, entries) in &groups {
            if let Some(archive) = archive {
                rows.push(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(theme::TEXT_DIM)
                        .child("▾")
                        .child(div().truncate().child(archive.archive_name().to_string()))
                        .into_any_element(),
                );
            }
            for (index, label, loaded) in entries {
                let is_active = Some(*index) == self.active;
                let indented = archive.is_some();
                let select_index = *index;
                let close_index = *index;
                let label_color = if is_active {
                    theme::TEXT_STRONG
                } else {
                    theme::TEXT
                };
                let dot_color = if *loaded { theme::ACCENT } else { theme::WARN };
                let mut row = div()
                    .id(("file-row", *index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .pr_2()
                    .py_1()
                    .cursor_pointer()
                    .when(is_active, |s| s.bg(theme::BG_SELECTED))
                    .hover(|s| s.bg(theme::BG_ROW_HOVER))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.switch_to(select_index, cx);
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(dot_color)
                            .child("●"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_color(label_color)
                            .child(label.clone()),
                    )
                    .child(
                        div()
                            .id(("file-row-close", *index))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::TEXT_STRONG))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |view, _, _, cx| {
                                    view.close_active_at(close_index, cx);
                                }),
                            )
                            .child("×"),
                    );
                row = if indented {
                    row.pl(px(f32::from(theme::FILE_PANEL_ENTRY_INDENT)))
                } else {
                    row.pl_2()
                };
                rows.push(row.into_any_element());
            }
        }

        div()
            .flex()
            .flex_col()
            .w(theme::FILE_PANEL_WIDTH)
            .h_full()
            .flex_shrink_0()
            .bg(theme::BG_PANEL)
            .border_r_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(f32::from(theme::TAB_BAR_HEIGHT)))
                    .px_2()
                    .border_b_1()
                    .border_color(theme::BORDER)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .child(format!("{} open", self.documents.len())),
                    )
                    .child(
                        div()
                            .id("file-panel-close")
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::TEXT_STRONG))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|view, _, _, cx| {
                                    view.file_panel_open = false;
                                    cx.notify();
                                }),
                            )
                            .child("×"),
                    ),
            )
            .child(
                div()
                    .id("file-panel-scroll")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .children(rows),
            )
    }

    /// Right-side column-inspector panel. Shows the selected column's
    /// inferred type and metrics, or a hint when nothing is selected.
    fn render_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_3()
            .h(theme::HEADER_HEIGHT)
            .border_b_1()
            .border_color(theme::BORDER)
            .text_color(theme::TEXT_STRONG)
            .child("Column")
            .child(
                div()
                    .id("inspector-close")
                    .cursor_pointer()
                    .text_color(theme::TEXT_DIM)
                    .hover(|s| s.text_color(theme::WARN))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_inspector(cx)))
                    .child("×"),
            );

        let body: AnyElement = match &self.inspector_stats {
            Some(stats) => render_column_stats(stats),
            None => div()
                .p_3()
                .text_color(theme::TEXT_DIM)
                .child(if self.inspector_column.is_some() {
                    "computing…"
                } else {
                    "select a cell"
                })
                .into_any_element(),
        };

        div()
            .flex_shrink_0()
            .w(theme::INSPECTOR_WIDTH)
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::BG_PANEL)
            .border_l_1()
            .border_color(theme::BORDER)
            .child(header)
            .child(
                div()
                    .id("inspector-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }

    fn render_status(&self) -> impl IntoElement {
        let (rows, cols, size) = match &self.model {
            Some(model) => (
                model.row_count(),
                model.doc.column_count(),
                model.doc.opened.len(),
            ),
            None => (0, 0, 0),
        };
        // When a filter is active the row count reflects the filtered
        // set; the original total is shown alongside it.
        let rows_label = match &self.filter_rows {
            Some(filtered) => format!(
                "{} rows (filtered from {})",
                format_count(filtered.len() as u64),
                format_count(rows)
            ),
            None => format!("{} rows", format_count(rows)),
        };
        let sort_label = self.sort_label();
        let state = match &self.state {
            ViewState::Empty => "idle".to_string(),
            ViewState::Loading(note) => format!("loading: {note}"),
            ViewState::Ready => "ready".to_string(),
            ViewState::Error(message) => format!("error: {message}"),
        };
        let delimiter = self
            .model
            .as_ref()
            .map(|m| super::describe_delimiter(m.doc.meta.delimiter))
            .unwrap_or("-");
        let warnings = self
            .model
            .as_ref()
            .map(|m| m.doc.meta.warnings.len())
            .unwrap_or(0);

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_4()
            .h(theme::STATUS_HEIGHT)
            .w_full()
            .px_3()
            .bg(theme::BG_STATUS)
            .border_t_1()
            .border_color(theme::BORDER)
            .text_xs()
            .text_color(theme::TEXT_DIM)
            .child(rows_label)
            .child(format!("{cols} cols"))
            .child(format!("delim {}", delimiter))
            .child(archive_picker::format_size(size))
            .when_some(sort_label, |this, label| {
                this.child(div().text_color(theme::ACCENT).child(label))
            })
            .when(warnings > 0, |this| {
                this.child(
                    div()
                        .text_color(theme::WARN)
                        .child(format!("{warnings} warnings")),
                )
            })
            .child(div().flex_1())
            .when_some(self.status_message.clone(), |this, msg| {
                // Highlight a copy-failed message red; otherwise the
                // default accent is enough to draw attention.
                let is_failure = msg.starts_with("copy failed");
                this.child(
                    div()
                        .text_color(if is_failure {
                            gpui::red()
                        } else {
                            theme::ACCENT.into()
                        })
                        .child(msg),
                )
            })
            .child(state)
    }

    /// Width of the columns area, computed synchronously from the window
    /// width minus every panel that shares the grid's flex row. All
    /// panels are `flex_shrink_0` with a fixed `theme` width, so this
    /// matches the measured layout exactly - but unlike the previous
    /// frame's `grid_bounds` it is correct on the same frame a panel is
    /// toggled, so closing the inspector / diagnostics / file panel no
    /// longer leaves a one- to two-frame blank gap before the table
    /// reflows into the freed space. `grid_bounds` is still recorded for
    /// drag auto-scroll, which needs the real on-screen rectangle.
    fn grid_viewport_width(&self, window: &Window) -> f32 {
        let mut width =
            f32::from(window.viewport_size().width) - f32::from(theme::ROW_NUMBER_WIDTH);
        if self.file_panel_open && !self.documents.is_empty() {
            width -= f32::from(theme::FILE_PANEL_WIDTH);
        }
        if self.inspector_open && self.model.is_some() {
            width -= f32::from(theme::INSPECTOR_WIDTH);
        }
        if self.diagnostics_open && self.model.is_some() {
            width -= f32::from(theme::DIAGNOSTICS_WIDTH);
        }
        width.max(0.0)
    }

    fn render_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(model) = self.model.clone() else {
            return div().into_any_element();
        };

        // A document just became ready: move keyboard focus onto the
        // grid so arrow navigation and Cmd/Ctrl+C work without a click.
        // Focus is moved on the next frame rather than mid-render.
        if self.focus_grid_on_next_frame {
            self.focus_grid_on_next_frame = false;
            let focus_handle = self.focus_handle.clone();
            cx.defer_in(window, move |_view, window, _cx| {
                window.focus(&focus_handle);
            });
        }

        // The viewport is computed synchronously (see
        // `grid_viewport_width`) rather than from the previous frame's
        // measured bounds, so a panel toggle reflows the grid in the
        // same frame it is toggled.
        let viewport_w = self.grid_viewport_width(window);
        self.viewport_w = viewport_w;

        // When a filter or sort is active the list renders that set; `i`
        // is a display position that maps through `view_rows` to a
        // record index. Without either the two are the same.
        let view_rows = self.view_rows();
        let row_count = view_rows
            .as_ref()
            .map_or(model.row_count() as usize, |rows| rows.len());
        let scroll_x = self.scroll_x;

        // The entity handle is needed inside the per-cell click handler
        // closures and the header click. We capture it here (where `cx`
        // is `&mut Context`) and move it into each row / header render.
        // Inside a click, `cx` is `&mut App`, and `entity.update(cx, ...)`
        // walks back up to the `MainView` so we can mutate view state
        // and call `cx.notify()`.
        let entity = cx.entity();

        let sort_indicator = self
            .sort_spec
            .map(|spec| (spec.column, spec.order == SortOrder::Descending));
        let header = grid::render_header(
            &model,
            scroll_x,
            viewport_w,
            self.search_column,
            sort_indicator,
            {
                let entity = entity.clone();
                move |col, ev, app_cx| {
                    // Alt/Option-click sets the search scope; a plain
                    // click cycles the sort.
                    let scope = ev.modifiers.alt;
                    entity.update(app_cx, |view, cx| {
                        if scope {
                            view.toggle_search_column(col, cx);
                        } else {
                            view.cycle_sort(col, cx);
                        }
                    });
                }
            },
            {
                let entity = entity.clone();
                move |col, ev, app_cx| {
                    let pointer_x = f32::from(ev.position.x);
                    entity.update(app_cx, |view, cx| {
                        let width = view
                            .model
                            .as_ref()
                            .map(|model| model.layout.widths.get(col).copied().unwrap_or(0.0))
                            .unwrap_or(0.0);
                        view.start_column_drag(col, pointer_x, width, cx);
                    });
                }
            },
        );

        let list = uniform_list(
            "anvil-grid",
            row_count,
            cx.processor(move |this, range: Range<usize>, _window, _cx| {
                let Some(model) = this.model.clone() else {
                    return Vec::new();
                };
                let scroll_x = this.scroll_x;
                let viewport_w = this.viewport_w;
                let selection = this.selection.clone();
                let copy_flash_rect = this.copy_flash_rect;
                let search_matches = this.search_matches.clone();
                let search_active = this.search_active;
                let search_query = this.search_query.text().to_string();
                let search_options = this.search_options();
                let view_rows = this.view_rows();
                let entity = entity.clone();
                let warning_rows = model.warnings.clone();
                let row_ctx = grid::RowContext {
                    selection: &selection,
                    scroll_x,
                    viewport_w,
                    copy_flash: copy_flash_rect,
                    search_matches: &search_matches,
                    search_active,
                    search_query: &search_query,
                    search_options,
                    warning_rows: warning_rows.as_ref(),
                };
                range
                    .map(|i| {
                        // Per-row closure: each row is built with its
                        // own clone of the entity handle. The
                        // cell-click closures produced by render_row
                        // are themselves Fn + Clone, so cloning
                        // again here is required by the FnMut
                        // contract on `range.map`.
                        let entity_for_row = entity.clone();
                        let record_index = match &view_rows {
                            Some(rows) => rows.get(i).copied().unwrap_or(0),
                            None => i as u64,
                        };
                        grid::render_row(
                            &model,
                            record_index,
                            &row_ctx,
                            {
                                let entity = entity_for_row.clone();
                                move |ev, row, col, app_cx| {
                                    let pos = CellPos::new(row, col);
                                    let extend = ev.modifiers.shift;
                                    let pointer = ev.position;
                                    entity.update(app_cx, |view, cx| {
                                        if extend {
                                            view.selection.extend_to(pos);
                                        } else {
                                            view.selection.set(pos);
                                        }
                                        // A press arms drag-select and
                                        // starts the edge auto-scroll
                                        // loop; the matching mouse-up
                                        // disarms both.
                                        view.drag_selecting = true;
                                        view.pointer_pos = Some(pointer);
                                        view.start_auto_scroll(cx);
                                        view.kick_inspector(cx);
                                        cx.notify();
                                    });
                                }
                            },
                            {
                                let entity = entity_for_row;
                                move |ev, row, col, app_cx| {
                                    let dragging = ev.dragging();
                                    let pointer = ev.position;
                                    let pos = CellPos::new(row, col);
                                    entity.update(app_cx, |view, cx| {
                                        view.pointer_pos = Some(pointer);
                                        if dragging && view.drag_selecting {
                                            view.selection.extend_to(pos);
                                            cx.notify();
                                        }
                                    });
                                }
                            },
                        )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .size_full()
        .track_scroll(self.v_scroll.clone());

        // Overlay canvas: records the scroll area's window-space bounds
        // and installs a global mouse-move listener. The listener sees
        // moves outside the grid, which per-cell handlers cannot, and is
        // what drives drag auto-scroll. The canvas has no hitbox, so it
        // does not intercept clicks or hover.
        let entity_for_canvas = cx.entity();
        let auto_scroll_overlay = canvas(
            |_, _, _| (),
            move |bounds, _, window, app_cx| {
                entity_for_canvas.update(app_cx, |view, cx| {
                    // Keep the grid's on-screen rectangle fresh for drag
                    // auto-scroll (the viewport width is computed
                    // synchronously, so a panel toggle no longer waits on
                    // this). No-op when the bounds are stable, so this
                    // cannot loop.
                    if view.grid_bounds != Some(bounds) {
                        view.grid_bounds = Some(bounds);
                        cx.notify();
                    }
                });
                let entity = entity_for_canvas.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, app_cx| {
                    entity.update(app_cx, |view, cx| {
                        view.pointer_pos = Some(ev.position);
                        if view.column_drag.is_some() {
                            view.column_drag_move(f32::from(ev.position.x), cx);
                        } else if view.drag_selecting {
                            view.auto_scroll_step(cx);
                        }
                    });
                });
                let entity = entity_for_canvas.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, _, _, app_cx| {
                    entity.update(app_cx, |view, cx| {
                        if view.column_drag.is_some() {
                            view.end_column_drag(cx);
                        }
                    });
                });
            },
        )
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .key_context("Grid")
                    // Test-only: lets UI tests look up the grid's pixel
                    // bounds to synthesise mouse coordinates. A no-op in
                    // release builds.
                    .debug_selector(|| "anvil-grid-list".to_string())
                    // Keyboard events are only routed to `on_key_down`
                    // while this element holds focus. `track_focus`
                    // also makes a mouse-down anywhere in the grid
                    // transfer focus, so clicking a cell then pressing
                    // Cmd/Ctrl+C works.
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::on_grid_key_down))
                    .on_scroll_wheel(cx.listener(Self::on_grid_scroll))
                    // End a drag-select on release, whether the pointer
                    // is still over the grid (bubble) or has left it
                    // (capture); stop the auto-scroll loop too.
                    .on_mouse_up(
                        gpui::MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.drag_selecting = false;
                            view.stop_auto_scroll();
                            view.kick_inspector(cx);
                        }),
                    )
                    .on_mouse_up_out(
                        gpui::MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.drag_selecting = false;
                            view.stop_auto_scroll();
                            view.kick_inspector(cx);
                        }),
                    )
                    .child(auto_scroll_overlay)
                    .child(list),
            )
            .child(self.render_h_scrollbar(cx))
            .into_any_element()
    }

    // ---- column resize ---------------------------------------------------

    /// Begin a column-resize drag from a header boundary. `pointer_x` is
    /// the press position and `width` the column's width at press time.
    fn start_column_drag(
        &mut self,
        column: usize,
        pointer_x: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        self.column_drag = Some((column, pointer_x, width));
        cx.notify();
    }

    /// Update the dragged column's width from the current pointer x. The
    /// width is clamped by `ColumnLayout::resized`, which also rebuilds
    /// the offsets and total so the header, body cells and horizontal
    /// scrollbar stay aligned.
    fn column_drag_move(&mut self, pointer_x: f32, cx: &mut Context<Self>) {
        let Some((column, start_x, start_width)) = self.column_drag else {
            return;
        };
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let new_width = start_width + (pointer_x - start_x);
        model.layout = Arc::new(model.layout.resized(column, new_width));
        cx.notify();
    }

    /// End a column-resize drag and persist the widths to the active tab.
    fn end_column_drag(&mut self, cx: &mut Context<Self>) {
        if self.column_drag.take().is_some() {
            self.snapshot_active();
            cx.notify();
        }
    }

    fn render_h_scrollbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (viewport, total) = match &self.model {
            Some(model) => (self.viewport_w, model.layout.total),
            None => (0.0, 0.0),
        };
        if total <= viewport || viewport <= 0.0 {
            return div().h(px(0.));
        }
        let frac = (viewport / total).clamp(0.05, 1.0);
        let thumb_w = frac * viewport;
        let max_scroll = (total - viewport).max(1.0);
        let thumb_left = (self.scroll_x / max_scroll).clamp(0.0, 1.0) * (viewport - thumb_w);
        let entity = cx.entity();

        div()
            .relative()
            .h(px(12.))
            .w_full()
            .bg(theme::BG_PANEL)
            .border_t_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(thumb_left))
                    .h(px(8.))
                    .w(px(thumb_w))
                    .rounded_full()
                    .bg(theme::BORDER_STRONG)
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |thumb_bounds, _, window, _| {
                                window.on_mouse_event({
                                    let entity = entity.clone();
                                    move |ev: &MouseDownEvent, _, _, cx| {
                                        if !thumb_bounds.contains(&ev.position) {
                                            return;
                                        }
                                        entity.update(cx, |view, _| {
                                            view.h_drag =
                                                Some((f32::from(ev.position.x), view.scroll_x));
                                        });
                                    }
                                });
                                window.on_mouse_event({
                                    let entity = entity.clone();
                                    move |_: &MouseUpEvent, _, _, cx| {
                                        entity.update(cx, |view, _| view.h_drag = None);
                                    }
                                });
                                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                                    if !ev.dragging() {
                                        return;
                                    }
                                    entity.update(cx, |view, cx| {
                                        let Some((start_x, start_scroll)) = view.h_drag else {
                                            return;
                                        };
                                        let Some(model) = &view.model else {
                                            return;
                                        };
                                        let viewport = view.viewport_w;
                                        let total = model.layout.total;
                                        if total <= viewport || viewport <= 0.0 {
                                            return;
                                        }
                                        let frac = (viewport / total).clamp(0.05, 1.0);
                                        let travel = (viewport - frac * viewport).max(1.0);
                                        let content_range = (total - viewport).max(1.0);
                                        let delta = f32::from(ev.position.x) - start_x;
                                        view.scroll_x = (start_scroll
                                            + delta * (content_range / travel))
                                            .clamp(0.0, content_range);
                                        cx.notify();
                                    });
                                });
                            },
                        )
                        .size_full(),
                    ),
            )
    }

    fn on_grid_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dx = match &event.delta {
            ScrollDelta::Pixels(point) => f32::from(point.x),
            ScrollDelta::Lines(point) => point.x * 20.0,
        };
        if dx != 0.0 {
            self.scroll_horizontal(-dx, cx);
        }
    }

    fn scroll_horizontal(&mut self, delta: f32, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let max = (model.layout.total - self.viewport_w).max(0.0);
        let next = (self.scroll_x + delta).clamp(0.0, max);
        if (next - self.scroll_x).abs() > f32::EPSILON {
            self.scroll_x = next;
            cx.notify();
        }
    }

    /// Scroll horizontally so `column` is fully visible, using the same
    /// layout offsets the grid renders with. Adds a small margin so the
    /// cell is not flush against the viewport edge. No-op when the
    /// column is already fully visible.
    fn ensure_column_visible(&mut self, column: usize, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let Some(&start) = model.layout.offsets.get(column) else {
            return;
        };
        let width = model.layout.widths.get(column).copied().unwrap_or(0.0);
        let viewport = self.viewport_w;
        if viewport <= 0.0 {
            return;
        }
        let margin = f32::from(theme::CELL_PAD_X) * 2.0;
        let left = self.scroll_x;
        let right = left + viewport;
        let mut next = left;
        if start - margin < left {
            next = start - margin;
        } else if start + width + margin > right {
            next = start + width + margin - viewport;
        }
        let max = (model.layout.total - viewport).max(0.0);
        let next = next.clamp(0.0, max);
        if (next - self.scroll_x).abs() > f32::EPSILON {
            self.scroll_x = next;
            cx.notify();
        }
    }

    /// Display position (0-based row in the rendered list) of a record
    /// index. With a filter or sort active the rendered order differs
    /// from record order, so scrolling by position must map through
    /// `view_rows`.
    fn display_position(&self, record: u64) -> usize {
        match self.view_rows() {
            Some(rows) => rows.iter().position(|&r| r == record).unwrap_or(0),
            None => record as usize,
        }
    }

    // ---- selection / keyboard navigation -------------------------------

    fn on_grid_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model.is_none() {
            return;
        }
        if let Some(action) = commands::action_for(&event.keystroke) {
            self.run_action(action, cx);
        }
    }

    /// Execute a keyboard [`Action`] decoded by `super::commands`. The
    /// binding table lives in `commands`; this is where it meets view
    /// state. New shortcuts add an `Action` variant and an arm here.
    fn run_action(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::MoveCursor {
                row_delta,
                column_delta,
                extend,
            } => self.move_selection(row_delta, column_delta, extend, cx),
            Action::Copy => self.copy_selection_to_clipboard(cx),
            Action::NextCell { forward } => self.move_cell_selection(forward, cx),
            Action::Escape => {
                // Close whichever bar is open first, so Esc always
                // dismisses the search / filter even when focus is on
                // the grid (the bar's own handler only runs while the
                // bar has focus).
                if self.search_open {
                    self.close_search();
                    self.focus_grid_on_next_frame = true;
                    cx.notify();
                } else if self.filter_open {
                    self.close_filter(cx);
                } else if self.ask_open {
                    self.close_ask(cx);
                } else if self.goto_open {
                    self.close_goto(cx);
                } else if !self.selection.is_empty() {
                    self.selection.clear();
                    cx.notify();
                } else if self.show_picker {
                    self.show_picker = false;
                    cx.notify();
                }
            }
            Action::OpenSearch => {
                if self.model.is_some() {
                    self.switch_to_search(cx);
                }
            }
            Action::NextMatch { direction } => {
                self.advance_match(direction, cx);
            }
            Action::ToggleFilePanel => {
                self.file_panel_open = !self.file_panel_open;
                cx.notify();
            }
            Action::OpenFilter => {
                if self.model.is_some() {
                    self.switch_to_filter(cx);
                }
            }
            Action::OpenAsk => {
                if self.model.is_some() {
                    self.switch_to_ask(cx);
                }
            }
            Action::ToggleInspector => self.toggle_inspector(cx),
            Action::ToggleDiagnostics => self.toggle_diagnostics(cx),
            Action::SelectAll => self.select_all(cx),
            Action::OpenGoToRow => self.open_goto(cx),
            Action::OpenFile => self.pick_file(cx),
        }
    }

    /// Switch to the filter bar, closing the search bar first so the two
    /// never fight over focus. Used by Cmd/Ctrl+L from the grid or the
    /// search bar.
    fn switch_to_filter(&mut self, cx: &mut Context<Self>) {
        if self.search_open {
            self.close_search();
            // `close_search` does not move focus; make sure a stale grid
            // focus request cannot steal it from the filter bar.
            self.focus_grid_on_next_frame = false;
        }
        if self.ask_open {
            self.close_ask(cx);
            self.focus_grid_on_next_frame = false;
        }
        if !self.filter_open {
            self.open_filter(cx);
        } else {
            cx.notify();
        }
    }

    /// Switch to the search bar, closing the filter bar first. Used by
    /// Cmd/Ctrl+F from the grid or the filter bar.
    fn switch_to_search(&mut self, cx: &mut Context<Self>) {
        if self.filter_open {
            self.close_filter(cx);
            // `close_filter` requests grid focus; hand focus to search.
            self.focus_grid_on_next_frame = false;
        }
        if self.ask_open {
            self.close_ask(cx);
            self.focus_grid_on_next_frame = false;
        }
        if !self.search_open {
            self.open_search(cx);
        } else {
            cx.notify();
        }
    }

    /// Switch to the natural-language Ask bar, closing the other bars.
    fn switch_to_ask(&mut self, cx: &mut Context<Self>) {
        if self.search_open {
            self.close_search();
            self.focus_grid_on_next_frame = false;
        }
        if self.filter_open {
            self.close_filter(cx);
            self.focus_grid_on_next_frame = false;
        }
        self.open_ask(cx);
    }

    /// Show or hide the column-inspector panel. Opening it kicks a stats
    /// scan for the currently selected column.
    fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_open = !self.inspector_open;
        if self.inspector_open {
            self.inspector_column = None;
            self.inspector_stats = None;
            self.kick_inspector(cx);
        }
        cx.notify();
    }

    /// Show or hide the diagnostics panel (Cmd/Ctrl+D). The parse
    /// warnings are already on the active document, but the UTF-8 scan
    /// is O(file) and runs on the background executor when the panel is
    /// opened; the report is cached until the document changes.
    fn toggle_diagnostics(&mut self, cx: &mut Context<Self>) {
        self.diagnostics_open = !self.diagnostics_open;
        if self.diagnostics_open {
            self.kick_encoding(cx);
        }
        cx.notify();
    }

    /// Cycle the active document's header interpretation:
    /// Auto -> Header -> Headerless -> Auto.
    fn cycle_header_mode(&mut self, cx: &mut Context<Self>) {
        let next = match self.header_mode {
            HeaderMode::Auto => HeaderMode::Header,
            HeaderMode::Header => HeaderMode::Headerless,
            HeaderMode::Headerless => HeaderMode::Auto,
        };
        self.apply_header_mode(next, cx);
    }

    /// Force a header interpretation and re-index the active document.
    /// The existing mmap / extracted temp file is reused (`reindex`),
    /// so this neither re-opens the file nor re-extracts an archive
    /// entry. Runs on the background executor with the same generation
    /// guard as the open tasks.
    fn apply_header_mode(&mut self, mode: HeaderMode, cx: &mut Context<Self>) {
        if self.header_mode == mode || self.model.is_none() {
            return;
        }
        let Some(active) = self.active else {
            return;
        };
        let id = self.documents[active].id;
        let doc = self.documents[active].doc.clone();
        let Some(doc) = doc else {
            return;
        };
        self.header_mode = mode;
        // Persist the mode now so a tab switch during the re-index keeps
        // it; the completed task also writes it back by tab id.
        self.snapshot_active();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let options = IndexOptions {
                header_mode: mode,
                ..IndexOptions::default()
            };
            let new_doc = executor.spawn(async move { reindex(&doc, options) }).await;
            this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.install_reindexed(id, Arc::new(new_doc), mode, cx);
            })
            .ok();
        });
        self.task = Some(task);
        cx.notify();
    }

    /// Replace the document for tab `id` with a freshly re-indexed one
    /// after a header-mode change. Row indexes shift when the header row
    /// is added or removed, so filter / sort / search results and an
    /// out-of-range selection are reset. Only the live fields are
    /// touched when the tab is still active.
    fn install_reindexed(
        &mut self,
        id: u64,
        doc: Arc<ParsedDocument>,
        mode: HeaderMode,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.documents.iter().position(|d| d.id == id) else {
            return;
        };
        if let Some(snap) = self.documents.get_mut(index) {
            snap.doc = Some(doc.clone());
            snap.header_mode = mode;
        }
        if self.active != Some(index) {
            cx.notify();
            return;
        }
        self.model = Some(GridModel::new(doc));
        self.invalidate_ask();
        self.filter_value_cache.clear();
        self.header_mode = mode;
        // The row set changed underneath these; drop stale results.
        self.filter_applied = None;
        self.filter_rows = None;
        self.filter_generation = self.filter_generation.wrapping_add(1);
        self.sort_spec = None;
        self.sort_rows = None;
        self.sort_generation = self.sort_generation.wrapping_add(1);
        self.search_matches.clear();
        self.search_active = -1;
        self.search_generation = self.search_generation.wrapping_add(1);
        if let Some(cursor) = self.selection.cursor() {
            let records = self.model.as_ref().map(|m| m.row_count()).unwrap_or(0);
            if cursor.record_index >= records {
                self.selection.clear();
            }
        }
        self.invalidate_encoding();
        self.kick_encoding(cx);
        self.focus_grid_on_next_frame = true;
        cx.notify();
    }

    /// Drop any cached inspector stats (the document changed).
    fn invalidate_inspector(&mut self) {
        self.inspector_column = None;
        self.inspector_stats = None;
        self.inspector_generation = self.inspector_generation.wrapping_add(1);
        self.inspector_task = None;
    }

    /// Drop the cached encoding report (the document changed). Bumping
    /// the generation also discards an in-flight scan's result.
    fn invalidate_encoding(&mut self) {
        self.encoding_report = None;
        self.encoding_generation = self.encoding_generation.wrapping_add(1);
        self.encoding_task = None;
    }

    /// Run the UTF-8 scan for the active document when the diagnostics
    /// panel is open and no report is cached. O(file length), so it runs
    /// on the background executor with the same generation guard as the
    /// filter / sort / inspector tasks. No-op when a report already
    /// exists for this document.
    fn kick_encoding(&mut self, cx: &mut Context<Self>) {
        if !self.diagnostics_open || self.encoding_report.is_some() {
            return;
        }
        let Some(doc) = self.model.as_ref().map(|m| m.doc.clone()) else {
            self.encoding_report = None;
            return;
        };
        self.encoding_generation = self.encoding_generation.wrapping_add(1);
        let generation = self.encoding_generation;
        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let report = executor.spawn(async move { scan_encoding(&doc) }).await;
            this.update(cx, |view, cx| {
                if view.encoding_generation != generation {
                    return;
                }
                view.encoding_report = Some(Arc::new(report));
                view.encoding_task = None;
                cx.notify();
            })
            .ok();
        });
        self.encoding_task = Some(task);
        cx.notify();
    }

    /// Recompute the selected column's stats when the inspector is open.
    /// No-op when the column has not changed since the last scan, so
    /// vertical navigation does not rescan. Runs on the background
    /// executor with a generation guard, like the filter/sort tasks.
    fn kick_inspector(&mut self, cx: &mut Context<Self>) {
        if !self.inspector_open {
            return;
        }
        let Some(column) = self.selection.cursor().map(|c| c.column_index) else {
            return;
        };
        if self.inspector_column == Some(column) && self.inspector_stats.is_some() {
            return;
        }
        let Some(doc) = self.model.as_ref().map(|m| m.doc.clone()) else {
            return;
        };
        self.inspector_column = Some(column);
        self.inspector_generation = self.inspector_generation.wrapping_add(1);
        let generation = self.inspector_generation;
        self.inspector_stats = None;

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let stats = executor
                .spawn(async move { column_stats(&doc, column, &StatsOptions::default()) })
                .await;
            this.update(cx, |view, cx| {
                if view.inspector_generation != generation {
                    return;
                }
                view.inspector_stats = Some(Arc::new(stats));
                view.inspector_task = None;
                cx.notify();
            })
            .ok();
        });
        self.inspector_task = Some(task);
        cx.notify();
    }

    /// Format the current selection as TSV and put it on the system
    /// clipboard. A status-bar message reports success or failure.
    /// Does nothing (and shows no message) when there is no selection.
    fn copy_selection_to_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let doc = model.doc.clone();
        let selection = self.selection.clone();
        let cell_at = |r: u64, c: usize| {
            doc.cell(r, anvil_core::ColumnIndex(c))
                .map(|cell| cell.to_exact_string().into_owned())
        };
        // Under an active filter/sort, copy only the rows the user can
        // actually see, in display order. Select-all relies on this so a
        // hidden record is never written to the clipboard.
        let (text, count) = match self.view_rows() {
            Some(rows) => (
                selection.copy_view_text_with(&rows, cell_at),
                selection.view_cell_count(&rows),
            ),
            None => (selection.copy_text_with(cell_at), selection.cell_count()),
        };
        if count == 0 {
            // Nothing to copy. We still notify so that any prior
            // "copied ..." message is cleared.
            self.status_message = None;
            cx.notify();
            return;
        }
        match (self.clipboard)(&text) {
            Ok(()) => {
                self.status_message = Some(format!("copied {count} cell{}", plural(count)));
                self.start_copy_flash(selection.normalize(), cx);
            }
            Err(err) => {
                self.status_message = Some(format!("copy failed: {err}"));
            }
        }
        cx.notify();
    }

    /// Paint `rect` green and schedule its clearing after
    /// `COPY_FLASH_DURATION`. A generation guard means a second copy
    /// during a flash restarts the timer instead of the first timer
    /// clearing the new flash early.
    fn start_copy_flash(&mut self, rect: Option<(CellPos, CellPos)>, cx: &mut Context<Self>) {
        let Some(rect) = rect else {
            return;
        };
        self.copy_flash_rect = Some(rect);
        self.copy_flash_generation = self.copy_flash_generation.wrapping_add(1);
        let generation = self.copy_flash_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPY_FLASH_DURATION).await;
            this.update(cx, |view, cx| {
                if view.copy_flash_generation == generation {
                    view.copy_flash_rect = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The record indexes in display order for the active tab: the
    /// sorted permutation when a sort is active, otherwise the filtered
    /// set, otherwise `None` (identity order). Sorting composes with
    /// filtering because `sort_rows` is built from `filter_rows`.
    fn view_rows(&self) -> Option<Arc<Vec<u64>>> {
        self.sort_rows.clone().or_else(|| self.filter_rows.clone())
    }

    /// Cycle the active tab's sort for `column`: none -> ascending ->
    /// descending -> none. Sorting a different column restarts at
    /// ascending. The numeric/text comparison kind is resolved from
    /// inferred column types when the background task runs.
    fn cycle_sort(&mut self, column: usize, cx: &mut Context<Self>) {
        let next = match self.sort_spec {
            Some(spec) if spec.column == column && spec.order == SortOrder::Ascending => {
                Some(SortSpec {
                    column,
                    order: SortOrder::Descending,
                    kind: spec.kind,
                })
            }
            Some(spec) if spec.column == column && spec.order == SortOrder::Descending => None,
            _ => Some(SortSpec {
                column,
                order: SortOrder::Ascending,
                kind: SortKind::Text,
            }),
        };
        self.sort_spec = next;
        self.kick_sort(cx);
    }

    /// Recompute the active tab's sort permutation on the background
    /// executor. `None` spec clears it. A generation guard discards
    /// stale results, mirroring the filter task.
    fn kick_sort(&mut self, cx: &mut Context<Self>) {
        let Some(spec) = self.sort_spec else {
            self.sort_rows = None;
            self.sort_generation = self.sort_generation.wrapping_add(1);
            self.sort_task = None;
            cx.notify();
            return;
        };
        let Some(doc) = self.model.as_ref().map(|m| m.doc.clone()) else {
            self.sort_rows = None;
            return;
        };
        // The base is whatever is currently visible: the filtered set,
        // or every record when there is no filter.
        let base: Vec<u64> = match &self.filter_rows {
            Some(rows) => (**rows).clone(),
            None => (0..doc.record_count()).collect(),
        };
        self.sort_generation = self.sort_generation.wrapping_add(1);
        let generation = self.sort_generation;
        // Show the unsorted base while the sort runs.
        self.sort_rows = None;

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let sorted = executor
                .spawn(async move {
                    let profiles = infer_schema(&doc);
                    let kind = profiles
                        .get(spec.column)
                        .map(|profile| SortKind::for_column_type(profile.column_type))
                        .unwrap_or(SortKind::Text);
                    sort_rows(&doc, &base, &SortSpec { kind, ..spec })
                })
                .await;
            this.update(cx, |view, cx| {
                if view.sort_generation != generation {
                    return;
                }
                view.sort_rows = Some(Arc::new(sorted));
                cx.notify();
            })
            .ok();
        });
        self.sort_task = Some(task);
        cx.notify();
    }

    /// Status-bar label for the active sort (column name + direction).
    fn sort_label(&self) -> Option<String> {
        let spec = self.sort_spec?;
        let name = self
            .model
            .as_ref()
            .map(|m| m.doc.meta.column_label(spec.column))
            .unwrap_or_else(|| format!("col{}", spec.column));
        let arrow = match spec.order {
            SortOrder::Ascending => "▲",
            SortOrder::Descending => "▼",
        };
        Some(format!("sorted by {name} {arrow}"))
    }

    /// Move the selection cursor. With no filter this is a straight
    /// record-index move (delegated to `Selection::move_cursor`). With
    /// a filter active, vertical movement walks the *visible* rows so
    /// the cursor never lands on a hidden record, and horizontal
    /// movement is unchanged.
    fn move_selection(&mut self, dr: i64, dc: i64, extend: bool, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let cols = model.doc.column_count();
        let rows = self.view_rows();
        let Some(rows) = rows else {
            let records = model.row_count();
            // If there is no selection yet, start at (0, 0) so the
            // first arrow key makes the cursor visible.
            self.selection.move_cursor(dr, dc, cols, records, extend);
            if let Some(cursor) = self.selection.cursor() {
                self.ensure_column_visible(cursor.column_index, cx);
            }
            self.kick_inspector(cx);
            cx.notify();
            return;
        };
        if rows.is_empty() {
            return;
        }
        let cursor = self.selection.cursor().unwrap_or(CellPos::new(0, 0));
        let col_max = cols.saturating_sub(1) as i64;
        let col = (cursor.column_index as i64)
            .saturating_add(dc)
            .clamp(0, col_max) as usize;
        // Locate the cursor's display position; if it is on a hidden
        // record (e.g. the filter changed under it), fall back to the
        // closest visible position at or before it.
        let position = rows
            .binary_search(&cursor.record_index)
            .unwrap_or_else(|insert| insert.saturating_sub(1));
        let next_position = (position as i64)
            .saturating_add(dr)
            .clamp(0, rows.len() as i64 - 1) as usize;
        let next = CellPos::new(rows[next_position], col);
        if extend {
            self.selection.extend_to(next);
        } else {
            self.selection.set(next);
        }
        self.ensure_column_visible(next.column_index, cx);
        self.kick_inspector(cx);
        cx.notify();
    }

    /// Select the whole view (Cmd/Ctrl+A). The rectangle is anchored at
    /// `(0, 0)` and its cursor is the document's last row / last column;
    /// an active filter or sort is honoured because the grid paints only
    /// visible rows and the copy path
    /// ([`copy_selection_to_clipboard`](Self::copy_selection_to_clipboard))
    /// walks the visible display order, so hidden records are neither
    /// highlighted nor copied.
    fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let records = model.row_count();
        let cols = model.doc.column_count();
        self.selection.select_all(records, cols);
        self.kick_inspector(cx);
        cx.notify();
    }

    /// Move the selection cursor to the adjacent cell in row-major
    /// order (Tab / Shift-Tab). Without a filter this delegates to
    /// `Selection::move_cell`, which wraps across row boundaries and
    /// around the document ends. With a filter active it walks the
    /// *visible* cells so Tab never lands on a hidden record.
    fn move_cell_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let cols = model.doc.column_count();
        let Some(rows) = self.view_rows() else {
            let records = model.row_count();
            self.selection.move_cell(forward, cols, records, false);
            if let Some(cursor) = self.selection.cursor() {
                self.ensure_column_visible(cursor.column_index, cx);
            }
            self.kick_inspector(cx);
            cx.notify();
            return;
        };
        let total = rows.len() as u64 * cols as u64;
        if total == 0 {
            return;
        }
        let cursor = self.selection.cursor().unwrap_or(CellPos::new(0, 0));
        let position = rows
            .binary_search(&cursor.record_index)
            .unwrap_or_else(|insert| insert.saturating_sub(1));
        let col = cursor.column_index.min(cols.saturating_sub(1));
        let linear = position as u64 * cols as u64 + col as u64;
        let next_linear = if forward {
            (linear + 1) % total
        } else {
            (linear + total - 1) % total
        };
        let next = CellPos::new(
            rows[(next_linear / cols as u64) as usize],
            (next_linear % cols as u64) as usize,
        );
        self.selection.set(next);
        self.ensure_column_visible(next.column_index, cx);
        self.kick_inspector(cx);
        cx.notify();
    }

    // ---- search ----------------------------------------------------------

    // ---- drag auto-scroll ------------------------------------------------
    /// Start the auto-scroll tick loop if it is not already running. Safe
    /// to call on every cell press.
    fn start_auto_scroll(&mut self, cx: &mut Context<Self>) {
        if self.auto_scroll_task.is_some() {
            return;
        }
        let task = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(AUTO_SCROLL_TICK).await;
            let keep_going = this
                .update(cx, |view, cx| {
                    if !view.drag_selecting {
                        return false;
                    }
                    view.auto_scroll_step(cx);
                    true
                })
                .unwrap_or(false);
            if !keep_going {
                break;
            }
        });
        self.auto_scroll_task = Some(task);
    }

    /// Stop the auto-scroll loop. Called on mouse-up so the tick does not
    /// linger for a frame after the drag ends.
    fn stop_auto_scroll(&mut self) {
        self.auto_scroll_task = None;
    }

    /// One auto-scroll step: if the pointer is outside the grid's top or
    /// bottom edge, scroll the list by one row and extend the selection
    /// to the row at that edge. A no-op otherwise.
    fn auto_scroll_step(&mut self, cx: &mut Context<Self>) {
        let (Some(bounds), Some(pointer)) = (self.grid_bounds, self.pointer_pos) else {
            return;
        };
        let Some(model) = &self.model else {
            return;
        };
        let records = model.row_count();
        if records == 0 {
            return;
        }
        let Some(direction) = auto_scroll_direction(pointer.y, bounds) else {
            return;
        };

        let row_h = f32::from(theme::ROW_HEIGHT);
        let visible_rows = (f32::from(bounds.size.height) / row_h).floor().max(1.0) as u64;
        let first_visible = self.v_scroll.0.borrow().base_handle.top_item() as u64;
        let last_visible = (first_visible + visible_rows - 1).min(records - 1);
        let cursor_col = self.selection.cursor().map(|c| c.column_index).unwrap_or(0);

        // Extend to the row currently at the dragged edge, and pin the
        // next row to the top so the view advances by one per tick.
        let (target_row, scroll_to_first) = match direction {
            AutoScrollDirection::Up => (first_visible, first_visible.saturating_sub(1)),
            AutoScrollDirection::Down => {
                let next = (first_visible + 1).min(records.saturating_sub(1));
                (last_visible, next)
            }
        };

        self.v_scroll
            .scroll_to_item_strict(scroll_to_first as usize, ScrollStrategy::Top);
        self.selection
            .extend_to(CellPos::new(target_row, cursor_col));
        cx.notify();
    }

    fn render_overlay(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // The search bar is a normal-flow child of the center area so
        // the grid shifts down rather than sliding under it (the
        // active-match row stays fully visible). Only the modal
        // archive picker is overlaid here.
        let mut out = Vec::new();
        if let Some(archive_picker) = self.render_archive_picker(cx) {
            out.push(archive_picker);
        }
        // The filter autocomplete popup is an overlay so it paints above
        // the grid rather than being covered by the next sibling.
        if let Some(suggestions) = self.render_filter_suggestions(cx) {
            out.push(suggestions);
        }
        out
    }

    fn render_archive_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let archive = self.archive.clone()?;
        if !self.show_picker {
            return None;
        }
        let (usable, _rejected) = archive_picker::partition(&archive.candidates);

        let mut rows: Vec<AnyElement> = Vec::new();
        for (ix, candidate) in usable.iter().enumerate() {
            let name = candidate.entry_name.clone();
            let on_click_name = name.clone();
            rows.push(
                div()
                    .id(("archive-entry", ix))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::BG_ROW_HOVER))
                    .child(div().text_color(theme::TEXT_STRONG).truncate().child(name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .child(archive_picker::format_size(candidate.uncompressed_size)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.switch_to_entry(&on_click_name, cx);
                    }))
                    .into_any_element(),
            );
        }

        let usable_count = usable.len();
        let mut children: Vec<AnyElement> = vec![
            div()
                .text_color(theme::TEXT_STRONG)
                .text_lg()
                .child(format!("{usable_count} files in archive"))
                .into_any_element(),
            div()
                .text_xs()
                .text_color(theme::TEXT_DIM)
                .child(format!(
                    "{}: every usable file already has a tab",
                    archive.archive_name()
                ))
                .into_any_element(),
        ];
        children.extend(rows);

        let skipped = archive_picker::skipped_entries(&archive.candidates);
        if !skipped.is_empty() {
            // The list can be long (e.g. every file carries __MACOSX
            // metadata); show the first few and summarise the rest so the
            // panel stays responsive and readable.
            const MAX_SKIPPED_SHOWN: usize = 40;
            children.push(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(theme::TEXT_DIM)
                    .child(format!("{} skipped entries", skipped.len()))
                    .into_any_element(),
            );
            for (ix, entry) in skipped.iter().take(MAX_SKIPPED_SHOWN).enumerate() {
                let name = entry.name.clone();
                let reason = entry.reason.clone();
                children.push(
                    div()
                        .id(("archive-skipped", ix))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .rounded_sm()
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_color(theme::TEXT_DIM)
                                .child(name),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme::WARN)
                                .child(reason),
                        )
                        .into_any_element(),
                );
            }
            if skipped.len() > MAX_SKIPPED_SHOWN {
                children.push(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_DIM)
                        .child(format!("…and {} more", skipped.len() - MAX_SKIPPED_SHOWN))
                        .into_any_element(),
                );
            }
        }

        Some(
            div()
                .absolute()
                .top(px(0.))
                .left(px(0.))
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x000000aa))
                .child(
                    div()
                        .id("archive-picker-panel")
                        .flex()
                        .flex_col()
                        .gap_2()
                        .w(px(560.))
                        .max_h(px(500.))
                        .overflow_y_scroll()
                        .p_4()
                        .rounded_lg()
                        .bg(theme::BG_PANEL)
                        .border_1()
                        .border_color(theme::BORDER_STRONG)
                        .children(children),
                )
                .into_any_element(),
        )
    }

    fn render_center(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // In a multi-document world, the active document always wins
        // when it is set. The Empty / Loading / Error states only
        // take over when no document is open yet; once the user has
        // at least one tab, errors on background tasks surface only
        // in the status bar.
        if self.active.is_some() {
            // Active tab with a loaded document: show the grid.
            // Active tab whose ZIP entry is still being extracted:
            // show a placeholder so the user knows the tab is
            // working, not blank.
            return match self.model.is_some() {
                true => self.render_grid(window, cx),
                false => div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .bg(theme::BG)
                    .child(div().text_color(theme::TEXT_STRONG).child("Loading…"))
                    .child(
                        div()
                            .text_color(theme::TEXT_DIM)
                            .child(self.selected_entry.clone().unwrap_or_default()),
                    )
                    .children(self.render_progress_bar())
                    .into_any_element(),
            };
        }
        match &self.state {
            ViewState::Empty => div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .bg(theme::BG)
                .child(
                    div()
                        .text_xl()
                        .text_color(theme::TEXT_STRONG)
                        .child("Anvil"),
                )
                .child(
                    div()
                        .text_color(theme::TEXT_DIM)
                        .child("Drop a CSV, TSV, or ZIP anywhere, or click Open."),
                )
                .into_any_element(),
            ViewState::Loading(note) => div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .bg(theme::BG)
                .child(div().text_color(theme::TEXT_STRONG).child("Loading…"))
                .child(div().text_color(theme::TEXT_DIM).child(note.clone()))
                .children(self.render_progress_bar())
                .into_any_element(),
            ViewState::Error(message) => div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .bg(theme::BG)
                .child(div().text_color(gpui::red()).child("Could not open file"))
                .child(
                    div()
                        .max_w(px(700.))
                        .text_color(theme::TEXT_DIM)
                        .child(message.clone()),
                )
                .into_any_element(),
            ViewState::Ready => match self.model.is_some() {
                true => self.render_grid(window, cx),
                false => div().flex_1().into_any_element(),
            },
        }
    }

    fn on_file_drop(
        &mut self,
        paths: &ExternalPaths,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every dropped path becomes its own tab. The first one
        // sets `Loading` state visibly; subsequent ones load in the
        // background and arrive as additional tabs once their
        // background tasks complete.
        let paths = paths.paths();
        if paths.is_empty() {
            return;
        }
        for path in paths {
            self.begin_open(path.clone(), cx);
        }
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Search bar deferred focus: when the user opens the bar via
        // Cmd/Ctrl+F we set `focus_search_on_next_frame`; the very
        // next render schedules a deferred `window.focus` onto the
        // search handle. This avoids mutating focus mid-frame.
        if self.focus_search_on_next_frame {
            self.focus_search_on_next_frame = false;
            let focus_handle = self.search_focus_handle.clone();
            cx.defer_in(window, move |_view, window, _cx| {
                window.focus(&focus_handle);
            });
        }
        // Same deferred-focus dance for the filter bar, so opening it
        // with Cmd/Ctrl+L moves the caret into the input.
        if self.focus_filter_on_next_frame {
            self.focus_filter_on_next_frame = false;
            let focus_handle = self.filter_focus_handle.clone();
            cx.defer_in(window, move |_view, window, _cx| {
                window.focus(&focus_handle);
            });
        }
        // Deferred focus for the natural-language Ask bar.
        if self.focus_ask_on_next_frame {
            self.focus_ask_on_next_frame = false;
            let focus_handle = self.ask_focus_handle.clone();
            cx.defer_in(window, move |_view, window, _cx| {
                window.focus(&focus_handle);
            });
        }
        // Deferred focus for the go-to-row bar.
        if self.focus_goto_on_next_frame {
            self.focus_goto_on_next_frame = false;
            let focus_handle = self.goto_focus_handle.clone();
            cx.defer_in(window, move |_view, window, _cx| {
                window.focus(&focus_handle);
            });
        }

        let center = self.render_center(window, cx);
        let search_bar = self.render_search_bar(window, cx);
        let filter_bar = self.render_filter_bar(window, cx);
        let ask_bar = self.render_ask_bar(window, cx);
        let goto_bar = self.render_goto_bar(window, cx);
        let overlays = self.render_overlay(cx);

        // The grid body and the optional file panel share a row. The
        // search and filter bars (when open) sit in the normal flex
        // column above the grid; this pushes the grid down by their
        // height instead of letting them overlap the top of the grid,
        // which previously hid the active-match highlight when a
        // match was at row 0.
        let body = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .overflow_hidden()
            .children(filter_bar)
            .children(search_bar)
            .children(ask_bar)
            .children(goto_bar)
            .child(center)
            .children(overlays);
        let panel = if self.file_panel_open && !self.documents.is_empty() {
            Some(self.render_file_panel(cx).into_any_element())
        } else {
            None
        };
        let inspector = if self.inspector_open && self.model.is_some() {
            Some(self.render_inspector(cx))
        } else {
            None
        };
        let diagnostics = if self.diagnostics_open && self.model.is_some() {
            Some(self.render_diagnostics(cx))
        } else {
            None
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::BG)
            .text_sm()
            .text_color(theme::TEXT)
            .on_drop(cx.listener(Self::on_file_drop))
            .child(self.render_toolbar(cx))
            // Tab strip below the toolbar so users can see and
            // switch between all open documents. Empty (zero-height)
            // when no documents are open.
            .child(self.render_tab_bar(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .overflow_hidden()
                    .when_some(panel, |this, panel| this.child(panel))
                    .child(body)
                    .when_some(diagnostics, |this, diagnostics| this.child(diagnostics))
                    .when_some(inspector, |this, inspector| this.child(inspector)),
            )
            .child(self.render_status())
    }
}

// ---- background helpers -------------------------------------------------

fn load_and_index_with_progress(
    path: &std::path::Path,
    progress: anvil_core::ProgressSink<'_>,
) -> OpenOutcome {
    match anvil_core::load_with_progress(path, Some(progress)) {
        Ok(LoadResult::Ready(document)) => OpenOutcome::Ready(document),
        Ok(LoadResult::NeedArchiveChoice(source)) => OpenOutcome::NeedArchiveChoice(source),
        Err(err) => OpenOutcome::Error(err.to_string()),
    }
}

fn extract_and_index_with_progress(
    source: &Arc<ArchiveSource>,
    entry: &str,
    progress: anvil_core::ProgressSink<'_>,
) -> OpenOutcome {
    match anvil_core::load_entry_with_progress(source, entry, Some(progress)) {
        Ok(document) => OpenOutcome::Ready(Box::new(document)),
        Err(err) => OpenOutcome::Error(err.to_string()),
    }
}

/// One label/value row in the column inspector.
fn metric_row(label: &str, value: String) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .gap_2()
        .text_xs()
        .child(
            div()
                .w(px(92.))
                .flex_shrink_0()
                .text_color(theme::TEXT_DIM)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .text_color(theme::TEXT_STRONG)
                .truncate()
                .child(value),
        )
        .into_any_element()
}

/// The inspector body for a computed column: metrics plus samples and a
/// frequent-values list.
fn render_column_stats(stats: &ColumnStats) -> AnyElement {
    let distinct = match stats.distinct {
        DistinctCount::Exact(n) => format!("{} (exact)", format_count(n)),
        DistinctCount::Approximate(n) => format!("{} (approx)", format_count(n)),
    };
    let option = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".to_string());

    let mut rows: Vec<AnyElement> = vec![
        metric_row("name", stats.name.clone()),
        metric_row("type", format!("{:?}", stats.column_type)),
        metric_row("rows", format_count(stats.total_rows)),
        metric_row("non-empty", format_count(stats.non_empty)),
        metric_row("empty", format_count(stats.empty)),
        metric_row("distinct", distinct),
        metric_row(
            "duplicates",
            stats
                .duplicate_count
                .map(format_count)
                .unwrap_or_else(|| "-".to_string()),
        ),
        metric_row("min", option(&stats.min)),
        metric_row("max", option(&stats.max)),
        metric_row("longest", option(&stats.longest)),
        metric_row("shortest", option(&stats.shortest)),
    ];

    if !stats.samples.is_empty() {
        rows.push(metric_row("samples", stats.samples.join(", ")));
    }
    if !stats.frequent.is_empty() {
        rows.push(
            div()
                .mt_2()
                .text_xs()
                .text_color(theme::TEXT_DIM)
                .child("frequent values")
                .into_any_element(),
        );
        for value in &stats.frequent {
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .flex_1()
                            .text_color(theme::TEXT)
                            .truncate()
                            .child(value.value.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::TEXT_DIM)
                            .child(format_count(value.count)),
                    )
                    .into_any_element(),
            );
        }
    }

    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .children(rows)
        .into_any_element()
}

fn display_path(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

fn format_count(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Toolbar label for the active header interpretation.
fn header_mode_label(mode: HeaderMode) -> &'static str {
    match mode {
        HeaderMode::Auto => "Headers: Auto",
        HeaderMode::Header => "Headers: On",
        HeaderMode::Headerless => "Headers: Off",
    }
}

/// Which way a drag past the grid's vertical edges should auto-scroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoScrollDirection {
    Up,
    Down,
}

/// Decide the auto-scroll direction from the pointer's window-space `y`
/// and the grid's window-space bounds. `None` when the pointer is within
/// (or on) the vertical bounds, so no scrolling is needed.
fn auto_scroll_direction(pointer_y: Pixels, bounds: Bounds<Pixels>) -> Option<AutoScrollDirection> {
    let top = bounds.origin.y;
    let bottom = bounds.origin.y + bounds.size.height;
    if pointer_y < top {
        Some(AutoScrollDirection::Up)
    } else if pointer_y > bottom {
        Some(AutoScrollDirection::Down)
    } else {
        None
    }
}

#[cfg(test)]
mod tests;

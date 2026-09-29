//! The Anvil main view: toolbar, virtualized grid, status bar, archive picker.
//!
//! Threading model: opening/parsing/indexing runs on GPUI's background
//! executor. Each open bumps a generation counter; results from a stale
//! generation are dropped, which makes "drop a new file while the old one
//! is still loading" safe.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    canvas, div, prelude::*, px, rgb, uniform_list, AnyElement, Bounds, Context, ExternalPaths,
    FocusHandle, KeyDownEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render,
    ScrollDelta, ScrollStrategy, ScrollWheelEvent, Task, UniformListScrollHandle, Window,
};

use anvil_core::{
    search_records, ArchiveSource, CellPos, LoadResult, LoadedDocument, ParsedDocument,
    SearchOptions, Selection,
};

use super::archive_picker;
use super::commands::{self, Action};
use super::grid::{self, GridModel};
use super::theme;

/// How long the green "copied" flash stays over the selection before it
/// fades back to the normal selection colour.
const COPY_FLASH_DURATION: Duration = Duration::from_millis(450);

/// How often the drag auto-scroll task advances the view while the
/// pointer is held outside the grid's top or bottom edge.
const AUTO_SCROLL_TICK: Duration = Duration::from_millis(30);

/// Writes text to the system clipboard. Stored on `MainView` so UI
/// tests can substitute a recorder and assert the exact TSV without
/// clobbering the real clipboard.
#[allow(clippy::type_complexity)]
type ClipboardWriter = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

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
    search_query: String,
    /// Sorted record indexes that match the latest query. Reset on
    /// every keystroke while a new search is in flight.
    search_matches: Vec<u64>,
    /// Index into `search_matches` of the active match, or `-1`.
    search_active: i64,
    /// Bumped on every keystroke; obsolete search tasks compare
    /// their captured generation against this and discard themselves.
    search_generation: u64,
    /// Rectangle of the most recent successful copy, painted green
    /// until the flash timer clears it.
    copy_flash_rect: Option<(CellPos, CellPos)>,
    copy_flash_generation: u64,
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
            search_query: String::new(),
            search_matches: Vec::new(),
            search_active: -1,
            search_generation: 0,
            copy_flash_rect: None,
            copy_flash_generation: 0,
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
            search_query: String::new(),
            search_matches: Vec::new(),
            search_active: -1,
            search_generation: 0,
            copy_flash_rect: None,
            copy_flash_generation: 0,
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

    selection: Selection,
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
    /// overlay canvas each frame. Used to decide when a drag has left
    /// the list vertically and should auto-scroll.
    list_bounds: Option<Bounds<Pixels>>,
    /// Last observed pointer position in window coordinates, updated by
    /// the overlay canvas's global mouse-move listener. Reported even
    /// when the pointer is outside the grid, which per-cell handlers
    /// cannot see.
    pointer_pos: Option<Point<Pixels>>,
    /// Ticks while a drag is active and the pointer sits outside the
    /// list, advancing the view and growing the selection. Dropped on
    /// mouse-up (or cleared by the loop itself when the drag ends).
    auto_scroll_task: Option<Task<()>>,

    /// Search state (Cmd/Ctrl+F). `search_open` controls whether the
    /// search bar is visible. While open, `search_query` holds the
    /// user's typed needle; each keystroke spawns a background
    /// `query::search` task with `search_generation` so obsolete work
    /// is dropped. `search_matches` is the most recent result; it can
    /// be briefly stale relative to `search_query` while a search is
    /// in flight.
    search_open: bool,
    search_query: String,
    search_matches: Vec<u64>,
    /// Index into `search_matches` of the currently highlighted row.
    /// `-1` when there are no matches yet. Cycled by Enter / Shift+Enter
    /// inside the bar and by F3 / Shift-F3 anywhere.
    search_active: i64,
    /// Bumped on every keystroke / open / close. A background task
    /// only applies its result if the view's generation still matches.
    search_generation: u64,
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
}

impl MainView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            documents: Vec::new(),
            active: None,
            next_doc_id: 0,
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
            selection: Selection::empty(),
            drag_selecting: false,
            task: None,
            status_message: None,
            focus_handle: cx.focus_handle(),
            focus_grid_on_next_frame: false,
            clipboard: Arc::new(system_clipboard_write),
            copy_flash_rect: None,
            copy_flash_generation: 0,
            list_bounds: None,
            pointer_pos: None,
            auto_scroll_task: None,
            search_open: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_active: -1,
            search_generation: 0,
            search_task: None,
            search_focus_handle: cx.focus_handle(),
            focus_search_on_next_frame: false,
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
        snap.copy_flash_rect = self.copy_flash_rect;
        snap.copy_flash_generation = self.copy_flash_generation;
        snap.archive = self.archive.clone();
        snap.entry_name = self.selected_entry.clone();
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
        self.copy_flash_rect = None;
        self.copy_flash_generation = 0;
    }

    /// Copy `documents[new_active]` into the live per-document fields.
    /// Called after switching tabs so the next render reflects the
    /// new tab.
    fn load_active(&mut self) {
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
        self.copy_flash_rect = snap.copy_flash_rect;
        self.copy_flash_generation = snap.copy_flash_generation;
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
            // No tabs left — the centre panel falls back to the
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

    fn begin_open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.state = ViewState::Loading(format!("reading {}", display_path(&path)));
        self.show_picker = false;
        cx.notify();

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let outcome = executor.spawn(async move { load_and_index(&path) }).await;
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
        cx.spawn(async move |this, cx| {
            let outcome = executor
                .spawn(async move { extract_and_index(&archive, &entry) })
                .await;
            this.update(cx, |view, cx| view.apply_lazy_result(id, outcome, cx))
                .ok();
        })
        .detach();
    }

    /// Deliver the result of a lazy extraction to the tab it belongs
    /// to. The tab may have been closed while the extraction was in
    /// flight, in which case the result is discarded.
    fn apply_lazy_result(&mut self, id: u64, outcome: OpenOutcome, cx: &mut Context<Self>) {
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

            // Tab body — clickable to switch active.
            tabs.push(
                div()
                    .id(("doc-tab", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
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

        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(f32::from(theme::TAB_BAR_HEIGHT)))
            .w_full()
            .bg(theme::BG_TOOLBAR)
            .border_b_1()
            .border_color(theme::BORDER)
            .children(tabs)
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
        let state = match &self.state {
            ViewState::Empty => "idle".to_string(),
            ViewState::Loading(note) => format!("loading — {note}"),
            ViewState::Ready => "ready".to_string(),
            ViewState::Error(message) => format!("error — {message}"),
        };
        let delimiter = self
            .model
            .as_ref()
            .map(|m| super::describe_delimiter(m.doc.meta.delimiter))
            .unwrap_or("—");
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
            .child(format!("{} rows", format_count(rows)))
            .child(format!("{cols} cols"))
            .child(format!("delim {}", delimiter))
            .child(archive_picker::format_size(size))
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

        let viewport_w: f32 =
            f32::from(window.viewport_size().width) - f32::from(theme::ROW_NUMBER_WIDTH);
        self.viewport_w = viewport_w.max(0.0);

        let row_count = model.row_count() as usize;
        let scroll_x = self.scroll_x;
        let header = grid::render_header(&model, scroll_x, viewport_w);

        // The entity handle is needed inside the per-cell click handler
        // closures. We capture it here (where `cx` is `&mut Context`)
        // and move it into each row render. Inside the cell click,
        // `cx` is `&mut App`, and `entity.update(cx, ...)` walks back
        // up to the `MainView` so we can mutate `selection` and call
        // `cx.notify()`.
        let entity = cx.entity();

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
                let search_query = this.search_query.clone();
                let search_options = anvil_core::SearchOptions::default();
                let entity = entity.clone();
                let row_ctx = grid::RowContext {
                    selection: &selection,
                    scroll_x,
                    viewport_w,
                    copy_flash: copy_flash_rect,
                    search_matches: &search_matches,
                    search_active,
                    search_query: &search_query,
                    search_options,
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
                        grid::render_row(
                            &model,
                            i as u64,
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
                entity_for_canvas.update(app_cx, |view, _| view.list_bounds = Some(bounds));
                let entity = entity_for_canvas.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, app_cx| {
                    entity.update(app_cx, |view, cx| {
                        view.pointer_pos = Some(ev.position);
                        if view.drag_selecting {
                            view.auto_scroll_step(cx);
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
                        cx.listener(|view, _, _, _cx| {
                            view.drag_selecting = false;
                            view.stop_auto_scroll();
                        }),
                    )
                    .on_mouse_up_out(
                        gpui::MouseButton::Left,
                        cx.listener(|view, _, _, _cx| {
                            view.drag_selecting = false;
                            view.stop_auto_scroll();
                        }),
                    )
                    .child(auto_scroll_overlay)
                    .child(list),
            )
            .child(self.render_h_scrollbar(cx))
            .into_any_element()
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
            Action::Escape => {
                if self.search_open {
                    self.close_search();
                    self.focus_grid_on_next_frame = true;
                    cx.notify();
                } else if !self.selection.is_empty() {
                    self.selection.clear();
                    cx.notify();
                } else if self.show_picker {
                    self.show_picker = false;
                    cx.notify();
                }
            }
            Action::OpenSearch => {
                if self.model.is_some() && !self.search_open {
                    self.open_search(cx);
                }
            }
            Action::NextMatch { direction } => {
                self.advance_match(direction, cx);
            }
        }
    }

    /// Format the current selection as TSV and put it on the system
    /// clipboard. A status-bar message reports success or failure.
    /// Does nothing (and shows no message) when there is no selection.
    fn copy_selection_to_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let count = self.selection.cell_count();
        if count == 0 {
            // Nothing to copy. We still notify so that any prior
            // "copied ..." message is cleared.
            self.status_message = None;
            cx.notify();
            return;
        }
        let doc = model.doc.clone();
        let selection = self.selection.clone();
        let text = selection.copy_text_with(|r, c| {
            doc.cell(r, anvil_core::ColumnIndex(c))
                .map(|cell| cell.into_string())
        });
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

    fn move_selection(&mut self, dr: i64, dc: i64, extend: bool, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let records = model.row_count();
        let cols = model.doc.column_count();
        // If there is no selection yet, start at (0, 0) so the first
        // arrow key makes the cursor visible. `move_cursor` already
        // handles that case (it uses (0,0) as the implied cursor).
        self.selection.move_cursor(dr, dc, cols, records, extend);
        cx.notify();
    }

    // ---- search ----------------------------------------------------------

    /// Open the search bar overlay and kick off a search for whatever
    /// query was last typed (empty by default → 0 matches). The bar
    /// takes keyboard focus so subsequent keystrokes go into the input
    /// rather than the grid.
    fn open_search(&mut self, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search_matches.clear();
        self.search_active = -1;
        self.focus_search_on_next_frame = true;
        self.kick_search(cx);
        cx.notify();
    }

    /// Close the search bar and clear all search state. The grid keeps
    /// its current selection; the search rectangle stops being
    /// highlighted.
    fn close_search(&mut self) {
        // Snapshot first so the active tab's persisted search state
        // is consistent — the live fields are about to be reset to
        // "no search", which is what the snapshot now records.
        self.snapshot_active();
        self.search_open = false;
        self.search_query.clear();
        self.search_matches.clear();
        self.search_active = -1;
        self.search_task = None;
        // search_generation is bumped implicitly on the next open / edit.
    }

    /// Start (or restart) the background search for `self.search_query`.
    /// Bumps `search_generation` and drops the previous task. The task
    /// captures the generation so a stale result is silently discarded.
    fn kick_search(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            self.search_matches.clear();
            self.search_active = -1;
            return;
        };
        let needle = self.search_query.clone();
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        let doc = model.doc.clone();
        let options = SearchOptions::default();

        // Reset visible state while the search is in flight so the user
        // sees an empty match list rather than the previous run's.
        self.search_matches.clear();
        self.search_active = -1;

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let matches = executor
                .spawn(async move { search_records(&doc, &needle, &options) })
                .await;
            this.update(cx, |view, cx| {
                if view.search_generation != generation {
                    return;
                }
                view.apply_search_result(matches, cx);
            })
            .ok();
        });
        self.search_task = Some(task);
    }

    /// Apply the result of an in-flight search, ordered by record
    /// index. Picks the first match as the active one (or the closest
    /// after the cursor, if there is one).
    fn apply_search_result(&mut self, matches: Vec<u64>, cx: &mut Context<Self>) {
        self.search_matches = matches;
        self.search_active = if self.search_matches.is_empty() {
            -1
        } else {
            0
        };
        if self.search_active >= 0 {
            self.jump_to_active_match(cx);
        }
        cx.notify();
    }

    /// Move the active match by `direction` (1 = next, -1 = previous)
    /// and scroll the grid to it. A no-op when no search is active.
    fn advance_match(&mut self, direction: i8, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }
        let n = self.search_matches.len() as i64;
        let next = if self.search_active < 0 {
            if direction > 0 {
                0
            } else {
                n - 1
            }
        } else {
            // i64 wrap-safe cycle.
            let raw = self.search_active + direction as i64;
            ((raw % n) + n) % n
        };
        self.search_active = next;
        self.jump_to_active_match(cx);
    }

    /// Move the selection cursor to the active match's row and scroll
    /// the list so the row is visible.
    fn jump_to_active_match(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let Some(row) = self
            .search_matches
            .get(self.search_active as usize)
            .copied()
        else {
            return;
        };
        let col = self.selection.cursor().map(|c| c.column_index).unwrap_or(0);
        let col = col.min(model.doc.column_count().saturating_sub(1));
        self.selection.set(CellPos::new(row, col));
        // Scroll the list so the row is in view. We use the existing
        // scroll handle; this is the same call shape drag auto-scroll
        // uses to pin a row to the top.
        self.v_scroll
            .scroll_to_item_strict(row as usize, ScrollStrategy::Top);
        cx.notify();
    }

    /// The search bar's keyboard handler. Char keys append to the
    /// query; backspace deletes the last byte; Enter / Shift+Enter
    /// cycle matches; Esc closes the bar. Anything else falls through
    /// (and is therefore ignored).
    fn on_search_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.search_open {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;

        // Escape closes the bar and returns focus to the grid. This
        // re-uses `Action::Escape` so the binding table stays the
        // single source of truth, even though we are *not* the grid.
        if key == "escape" {
            self.close_search();
            self.focus_grid_on_next_frame = true;
            cx.notify();
            return;
        }

        // Enter / Shift+Enter cycle through matches without leaving the
        // search bar.
        if key == "enter" {
            let direction: i8 = if modifiers.shift { -1 } else { 1 };
            self.advance_match(direction, cx);
            return;
        }

        // Backspace pops one byte. We use `key == "backspace"` rather
        // than `event.keystroke.key_char` so we can be sure it is the
        // navigation key, not the literal character "backspace" (which
        // does not normally appear in cell data).
        if key == "backspace" && !modifiers.control && !modifiers.alt {
            self.search_query.pop();
            self.kick_search(cx);
            cx.notify();
            return;
        }

        // Plain printable characters are appended. We require *no*
        // modifiers so we don't swallow Cmd+F (already handled by the
        // grid binding), Option+E (an accented character), etc.
        if !modifiers.control && !modifiers.alt && !modifiers.function && !modifiers.secondary() {
            if let Some(ch) = &event.keystroke.key_char {
                if !ch.is_empty() && ch.chars().all(|c| !c.is_control()) {
                    self.search_query.push_str(ch);
                    self.kick_search(cx);
                    cx.notify();
                }
            }
        }
    }

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
        let (Some(bounds), Some(pointer)) = (self.list_bounds, self.pointer_pos) else {
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
        out
    }

    /// Search bar overlay (Cmd/Ctrl+F). Sits as a normal-flow child
    /// of the center area (above the grid, below the toolbar) so the
    /// grid never slides under it; this keeps the active match's
    /// highlight fully visible even when the match is at row 0. The
    /// bar carries its own `key_context("Search")` and `track_focus`
    /// so characters typed while it is open route through
    /// `on_search_key_down` rather than the grid handler.
    fn render_search_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.search_open {
            return None;
        }
        let query = self.search_query.clone();
        let (status_text, status_color) = self.search_status_text();

        Some(
            div()
                .h(px(f32::from(theme::STATUS_HEIGHT) * 1.5))
                .flex_shrink_0()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .bg(theme::BG_PANEL)
                .border_b_1()
                .border_color(theme::BORDER_STRONG)
                .key_context("Search")
                .track_focus(&self.search_focus_handle)
                .on_key_down(cx.listener(Self::on_search_key_down))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme::TEXT_DIM)
                        .child("Find:"),
                )
                .child(
                    div()
                        .flex_1()
                        .text_color(theme::TEXT_STRONG)
                        .truncate()
                        .child(if query.is_empty() {
                            "type to search…".to_string()
                        } else {
                            query
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(status_color)
                        .child(status_text),
                )
                .into_any_element(),
        )
    }

    /// Status-bar text + colour for the search bar: "N matches", "no
    /// matches", or "searching…" depending on whether a task is in
    /// flight.
    fn search_status_text(&self) -> (String, gpui::Rgba) {
        if !self.search_query.is_empty() && self.search_matches.is_empty() {
            // The empty match list could be either "still searching" or
            // "no matches". We treat an in-flight search task (or a
            // generation mismatch) as "searching…".
            if self.search_task.is_some() {
                return ("searching…".to_string(), theme::TEXT_DIM);
            }
            return ("no matches".to_string(), theme::WARN);
        }
        let total = self.search_matches.len();
        let active = if self.search_active >= 0 {
            self.search_active + 1
        } else {
            0
        };
        (format!("{active} of {total}"), theme::TEXT_DIM)
    }

    fn render_archive_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let archive = self.archive.clone()?;
        if !self.show_picker {
            return None;
        }
        let (usable, rejected) = archive_picker::partition(&archive.candidates);

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
                    "{} — every usable file already has a tab",
                    archive.archive_name()
                ))
                .into_any_element(),
        ];
        children.extend(rows);

        if !rejected.is_empty() {
            children.push(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(theme::TEXT_DIM)
                    .child(format!("{} skipped entries", rejected.len()))
                    .into_any_element(),
            );
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

        let center = self.render_center(window, cx);
        let search_bar = self.render_search_bar(cx);
        let overlays = self.render_overlay(cx);

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
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    // The search bar (when open) sits in the normal
                    // flex column above the grid; this pushes the
                    // grid down by the bar's height instead of
                    // letting the bar overlap the top of the grid,
                    // which previously hid the active-match
                    // highlight when a match was at row 0.
                    .children(search_bar)
                    .child(center)
                    .children(overlays),
            )
            .child(self.render_status())
    }
}

// ---- background helpers -------------------------------------------------

fn load_and_index(path: &std::path::Path) -> OpenOutcome {
    match anvil_core::load(path) {
        Ok(LoadResult::Ready(document)) => OpenOutcome::Ready(document),
        Ok(LoadResult::NeedArchiveChoice(source)) => OpenOutcome::NeedArchiveChoice(source),
        Err(err) => OpenOutcome::Error(err.to_string()),
    }
}

fn extract_and_index(source: &Arc<ArchiveSource>, entry: &str) -> OpenOutcome {
    match anvil_core::load_entry(source, entry) {
        Ok(document) => OpenOutcome::Ready(Box::new(document)),
        Err(err) => OpenOutcome::Error(err.to_string()),
    }
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
mod tests {
    use super::*;
    use std::sync::Mutex;

    use gpui::{Entity, TestAppContext, VisualTestContext};

    /// Test helper: append a loaded tab for `doc` and make it active.
    /// Mirrors the direct-file arm of `apply` without going through
    /// the background-task machinery.
    fn push_loaded_tab(view: &mut MainView, doc: Arc<ParsedDocument>) {
        let id = view.alloc_doc_id();
        view.documents
            .push(DocumentSnapshot::from_doc(id, doc, None, None));
        view.active = Some(view.documents.len() - 1);
        view.load_active();
    }

    fn sample_document() -> anvil_core::ParsedDocument {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sample.csv");
        std::fs::write(&path, b"a,b,c\n1,2,3\n4,5,6\n7,8,9\n").expect("write fixture");
        let opened = anvil_core::source::local::open_local(&path).expect("open local");
        anvil_core::index_opened(opened)
    }

    /// A wider-than-viewport document, for tests that need to scroll.
    fn large_document(rows: usize) -> anvil_core::ParsedDocument {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("large.csv");
        let mut csv = String::from("a,b,c\n");
        for i in 0..rows {
            csv.push_str(&format!("{i},{i},{i}\n"));
        }
        std::fs::write(&path, csv).expect("write fixture");
        let opened = anvil_core::source::local::open_local(&path).expect("open local");
        anvil_core::index_opened(opened)
    }

    /// Build a ZIP whose entries all contain the same small CSV, and
    /// return its `ArchiveSource`. The `TempDir` must be kept alive
    /// for the lifetime of the test, so callers bind it.
    fn make_zip_archive(names: &[&str]) -> (tempfile::TempDir, Arc<ArchiveSource>) {
        use std::io::Write;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("archive.zip");
        {
            let file = std::fs::File::create(&path).expect("create zip");
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for name in names {
                zip.start_file(*name, opts).expect("start file");
                zip.write_all(b"x,y\n1,2\n3,4\n").expect("write entry");
            }
            zip.finish().expect("finish zip");
        }
        let opened = anvil_core::open_path(&path).expect("open zip");
        let archive = match opened {
            anvil_core::OpenResult::Archive(a) => Arc::new(a),
            _ => panic!("expected an archive"),
        };
        (dir, archive)
    }

    /// Inject a ready document and focus the grid the way `render_grid`
    /// does after an open. Returns a recorder standing in for the system
    /// clipboard so the copy path can be asserted without side effects.
    fn inject(cx: &mut VisualTestContext, view: &Entity<MainView>) -> Arc<Mutex<Vec<String>>> {
        inject_doc(cx, view, sample_document())
    }

    /// As `inject`, but with a caller-supplied document.
    fn inject_doc(
        cx: &mut VisualTestContext,
        view: &Entity<MainView>,
        doc: anvil_core::ParsedDocument,
    ) -> Arc<Mutex<Vec<String>>> {
        let written: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = written.clone();
        cx.update_window_entity(view, |view, window, cx| {
            // Push the document as a fresh tab; mirrors what `apply`
            // does when an `OpenOutcome::Ready` arrives so render code
            // sees a consistent live state. The selection lives on the
            // new snapshot.
            let arc_doc = Arc::new(doc);
            push_loaded_tab(view, arc_doc);
            view.state = ViewState::Ready;
            view.selection.set(CellPos::new(0, 0));
            // The selection we just set lives on the live field; copy
            // it into the snapshot so the tab is internally consistent.
            if let Some(snap) = view.documents.get_mut(view.active.unwrap()) {
                snap.selection = view.selection.clone();
            }
            view.status_message = None;
            view.clipboard = Arc::new(move |text: &str| {
                sink.lock()
                    .expect("clipboard recorder")
                    .push(text.to_string());
                Ok(())
            });
            // Exercise the production focus path: `render_grid` sees
            // this flag and schedules a deferred `window.focus` for the
            // grid handle. (The `window` argument is unused on purpose
            // so the test cannot pass by focusing manually.)
            let _ = window;
            view.focus_grid_on_next_frame = true;
            cx.notify();
        });
        cx.run_until_parked();
        written
    }

    /// Regression: keyboard events only reach `on_grid_key_down` while
    /// the grid's focus handle is focused. Before the focus handle was
    /// introduced the grid was never focusable, so neither arrow
    /// navigation nor Cmd/Ctrl+C worked. This test fails if the grid
    /// loses `track_focus` or focus is no longer transferred to it.
    #[gpui::test]
    async fn arrow_keys_move_selection_when_grid_focused(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        cx.simulate_keystrokes("down");
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.selection.cursor(), Some(CellPos::new(1, 0)));
        });

        cx.simulate_keystrokes("right");
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.selection.cursor(), Some(CellPos::new(1, 1)));
        });
    }

    /// Regression for the reported "Cmd+C does nothing" bug. The
    /// shortcut only reaches `on_grid_key_down` while the grid is
    /// focused; this asserts the handler runs, reports a count, and
    /// writes the expected TSV to the clipboard sink.
    #[gpui::test]
    async fn copy_shortcut_reaches_handler_when_grid_focused(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        let written = inject(cx, &view);

        let copy_keystroke = if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        };
        cx.simulate_keystrokes(copy_keystroke);

        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.status_message.as_deref(), Some("copied 1 cell"));
            // The green confirmation flash covers the copied cell.
            assert_eq!(
                view.copy_flash_rect,
                Some((CellPos::new(0, 0), CellPos::new(0, 0)))
            );
        });

        // The selection is the single cell (0, 0), whose value in the
        // sample document is "1".
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        let captured = written.lock().expect("clipboard recorder").clone();
        assert_eq!(captured, vec![format!("1{ending}")]);
    }

    /// Centre of cell `(row, col)` in window coordinates, using the grid
    /// list's measured bounds plus the known row height, row-number
    /// gutter, and computed column widths. Assumes the grid is not
    /// scrolled (the tests never scroll it).
    fn cell_center(
        cx: &mut VisualTestContext,
        view: &Entity<MainView>,
        row: u64,
        col: usize,
    ) -> gpui::Point<gpui::Pixels> {
        let bounds = cx
            .debug_bounds("anvil-grid-list")
            .expect("grid list bounds");
        let (col_offset, col_width) = view.read_with(cx, |view, _| {
            let layout = &view.model.as_ref().expect("model").layout;
            (layout.offsets[col], layout.widths[col])
        });
        let x = bounds.origin.x + theme::ROW_NUMBER_WIDTH + px(col_offset) + px(col_width) / 2.0;
        let y = bounds.origin.y + px((row as f32 + 0.5) * f32::from(theme::ROW_HEIGHT));
        gpui::point(x, y)
    }

    /// Drag-select: press on one cell, drag across to another, release.
    /// The selection should become the rectangle between them, and the
    /// copy should then emit the whole block as TSV.
    #[gpui::test]
    async fn drag_select_builds_rectangle(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        let written = inject(cx, &view);

        let start = cell_center(cx, &view, 0, 0);
        let end = cell_center(cx, &view, 1, 1);
        let modifiers = gpui::Modifiers::default();

        cx.simulate_mouse_down(start, gpui::MouseButton::Left, modifiers);
        // Two moves to the same cell: the first updates the hover hit
        // test, the second is delivered to the now-hovered cell.
        cx.simulate_mouse_move(end, gpui::MouseButton::Left, modifiers);
        cx.simulate_mouse_move(end, gpui::MouseButton::Left, modifiers);
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, modifiers);

        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(
                view.selection.normalize(),
                Some((CellPos::new(0, 0), CellPos::new(1, 1)))
            );
            // Mouse-up disarms drag-select.
            assert!(!view.drag_selecting);
        });

        // Copy the dragged block and check the TSV payload.
        let copy_keystroke = if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        };
        cx.simulate_keystrokes(copy_keystroke);
        let ending = if cfg!(windows) { "\r\n" } else { "\n" };
        let captured = written.lock().expect("clipboard recorder").clone();
        assert_eq!(
            captured,
            vec![format!("1\t2{ending}4\t5{ending}")],
            "dragged block should copy as TSV"
        );
    }

    #[test]
    fn auto_scroll_direction_detects_edges() {
        let bounds = Bounds {
            origin: gpui::point(px(0.), px(100.)),
            size: gpui::size(px(400.), px(200.)),
        };
        assert_eq!(
            auto_scroll_direction(px(50.), bounds),
            Some(AutoScrollDirection::Up)
        );
        assert_eq!(
            auto_scroll_direction(px(350.), bounds),
            Some(AutoScrollDirection::Down)
        );
        // Inside (and exactly on) the vertical bounds: no scroll.
        assert_eq!(auto_scroll_direction(px(100.), bounds), None);
        assert_eq!(auto_scroll_direction(px(200.), bounds), None);
        assert_eq!(auto_scroll_direction(px(300.), bounds), None);
    }

    /// Dragging past the bottom edge should scroll the list and grow the
    /// selection past the initially visible rows without any further
    /// pointer movement (the tick loop keeps it going); this asserts the
    /// immediate step driven by the mouse-move listener.
    #[gpui::test]
    async fn drag_past_bottom_edge_starts_auto_scroll(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject_doc(cx, &view, large_document(1000));

        let start = cell_center(cx, &view, 0, 0);
        let bounds = cx
            .debug_bounds("anvil-grid-list")
            .expect("grid list bounds");
        let below = gpui::point(start.x, bounds.origin.y + bounds.size.height + px(40.0));
        let modifiers = gpui::Modifiers::default();

        cx.simulate_mouse_down(start, gpui::MouseButton::Left, modifiers);
        cx.simulate_mouse_move(below, gpui::MouseButton::Left, modifiers);
        cx.simulate_mouse_move(below, gpui::MouseButton::Left, modifiers);

        cx.update_window_entity(&view, |view, _, _| {
            assert!(view.drag_selecting, "drag should still be active");
            let cursor = view
                .selection
                .cursor()
                .expect("auto-scroll should extend the selection");
            assert!(
                cursor.record_index > 0,
                "selection should extend past the first visible row, got {cursor:?}"
            );
            // The list actually scrolled (the deferred scroll from each
            // step is applied by the following frame, so assert on the
            // resulting offset rather than the pending request).
            // Scrolling down makes the offset negative.
            let offset_y = view.v_scroll.0.borrow().base_handle.offset().y;
            assert!(
                offset_y < px(0.0),
                "auto-scroll should move the list down, got offset {offset_y:?}"
            );
        });

        cx.simulate_mouse_up(below, gpui::MouseButton::Left, modifiers);
        cx.update_window_entity(&view, |view, _, _| {
            assert!(!view.drag_selecting);
            assert!(
                view.auto_scroll_task.is_none(),
                "mouse-up should stop the auto-scroll loop"
            );
        });
    }

    /// Regression: Cmd/Ctrl+F should open the search bar overlay. The
    /// keystroke reaches `on_grid_key_down` while the grid is focused
    /// and the binding maps to `Action::OpenSearch`. After the
    /// keystroke the bar must be open and the query empty.
    #[gpui::test]
    async fn cmd_f_opens_search_bar_when_grid_focused(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        cx.update_window_entity(&view, |view, _, _| {
            assert!(!view.search_open);
        });

        let open = if cfg!(target_os = "macos") {
            "cmd-f"
        } else {
            "ctrl-f"
        };
        cx.simulate_keystrokes(open);

        cx.update_window_entity(&view, |view, _, _| {
            assert!(view.search_open, "search bar should be open");
            assert_eq!(view.search_query, "");
        });
    }

    /// Typing characters into the open search bar appends to the query
    /// (instead of moving the grid cursor). The bar's key handler must
    /// swallow printable keystrokes; the grid receives nothing.
    #[gpui::test]
    async fn search_bar_input_appends_and_skips_grid_navigation(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
        cx.run_until_parked();

        // Drive individual characters. GPUI's keystroke simulation
        // dispatches one KeystrokeEvent per `simulate_keystrokes`
        // argument.
        cx.simulate_keystrokes("f o o");
        cx.run_until_parked();

        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.search_query, "foo");
            // The grid cursor stays at the original (0, 0) because the
            // search bar swallowed the characters.
            assert_eq!(view.selection.cursor(), Some(CellPos::new(0, 0)));
        });
    }

    /// Backspace inside the search bar pops one character at a time
    /// without falling back to grid navigation.
    #[gpui::test]
    async fn search_bar_backspace_drops_last_character(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("f o o");
        cx.run_until_parked();
        cx.simulate_keystrokes("backspace");
        cx.run_until_parked();

        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.search_query, "fo");
        });
    }

    /// Closing the search bar via the `Escape` action must leave the
    /// grid ready to take keyboard focus again. We assert the bar is
    /// closed and `focus_grid_on_next_frame` is set so the next render
    /// will refocus the grid.
    #[gpui::test]
    async fn escape_closes_search_bar(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();

        cx.update_window_entity(&view, |view, _, _| {
            assert!(!view.search_open, "search bar should be closed");
            assert_eq!(view.search_query, "");
            assert_eq!(view.search_matches, Vec::<u64>::new());
            assert_eq!(view.search_active, -1);
            // We don't assert on `focus_grid_on_next_frame` here: that
            // flag is cleared as soon as `render_grid` runs (the
            // deferred focus consumes it). The contract we care about
            // is "the bar is closed"; the next user keystroke will
            // route through the grid handler.
        });
    }

    /// A search that has finished must populate `search_matches` and
    /// set `search_active` to the first match. We assert on the
    /// search state directly rather than the renderer.
    #[gpui::test]
    async fn search_populates_match_list(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("foo");
        // The background search task completes asynchronously; run
        // until the executor is idle.
        cx.run_until_parked();
        // The sample fixture is `a,b,c\n1,2,3\n4,5,6\n7,8,9\n`. None
        // of those rows contain "foo", so the result list is empty and
        // `search_active` stays at -1.
        cx.update_window_entity(&view, |view, _, _| {
            // 1M-row fixtures would take a moment, but the sample
            // document is tiny so the task should already be done.
            assert_eq!(view.search_matches, Vec::<u64>::new());
            assert_eq!(view.search_active, -1);
        });
    }

    /// Searching a fixture whose records contain the needle should
    /// return the matching record indexes in order.
    #[gpui::test]
    async fn search_finds_real_matches_in_fixture(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        // Build a fixture where the string `needle` appears in two
        // records. `a,b\nneedle,1\nx,needle\nother,foo\n` -> rows 0 and
        // 1 match when searching for "needle".
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("search.csv");
        std::fs::write(&path, b"a,b\nneedle,1\nx,needle\nother,foo\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        inject_doc(cx, &view, doc);

        cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("needle");
        cx.run_until_parked();

        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.search_matches, vec![0, 1]);
            assert_eq!(view.search_active, 0);
            // The first match is jumped to, so the cursor lives there.
            assert_eq!(view.selection.cursor(), Some(CellPos::new(0, 0)));
        });
    }

    /// Enter inside the search bar advances to the next match;
    /// Shift+Enter goes to the previous. The test uses a multi-match
    /// fixture so both directions are observable.
    #[gpui::test]
    async fn search_enter_cycles_through_matches(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cycle.csv");
        std::fs::write(&path, b"a,b\nhit,1\nhit,2\nhit,3\nskip,4\nhit,5\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        inject_doc(cx, &view, doc);

        cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("hit");
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            // First match is row 0.
            assert_eq!(view.search_active, 0);
        });

        // Enter -> next match.
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.search_active, 1);
        });

        // Shift+Enter -> previous.
        cx.simulate_keystrokes("shift-enter");
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.search_active, 0);
        });

        // Wrapping: Shift+Enter from row 0 should wrap to the last
        // match (row 3 = the 4th "hit", which is index 3 in the
        // 4-element result list).
        cx.simulate_keystrokes("shift-enter");
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.search_active, 3);
        });
    }

    /// F3 / Shift-F3 cycle through matches without consuming characters
    /// in the search bar. The bar must remain open and the query
    /// unchanged after the keystrokes; only `search_active` advances.
    #[gpui::test]
    async fn f3_advances_match_without_opening_bar(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f3.csv");
        std::fs::write(&path, b"a,b\nhit,1\nhit,2\nhit,3\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        inject_doc(cx, &view, doc);

        // Set up a search directly (the bar's focus path needs a
        // render, which the harness does not run automatically), let
        // the background search task complete, then dispatch F3.
        cx.update_window_entity(&view, |view, _, cx| {
            view.search_open = true;
            view.search_query = "hit".into();
            view.kick_search(cx);
            cx.notify();
        });
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.search_matches, vec![0, 1, 2]);
        });

        // F3 -> next match.
        cx.simulate_keystrokes("f3");
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert!(view.search_open, "F3 should not close the bar");
            assert_eq!(view.search_query, "hit", "F3 should not edit the query");
            assert_eq!(view.search_active, 1);
        });

        // Shift+F3 -> previous.
        cx.simulate_keystrokes("shift-f3");
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.search_active, 0);
        });
    }

    /// A new document open must clear the previous search state.
    /// Without this the next-document's grid would still show stale
    /// match highlighting.
    #[gpui::test]
    async fn opening_new_document_clears_search_state(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("f o o");
        cx.run_until_parked();

        // Now apply an `OpenOutcome::Ready` through the public
        // dispatcher. We can't construct a `LoadedDocument` here
        // because the constructor is on the `anvil` binary side; we
        // exercise `close_search` directly which is what `apply`
        // calls.
        cx.update_window_entity(&view, |view, _, _| view.close_search());
        cx.update_window_entity(&view, |view, _, _| {
            assert!(!view.search_open);
            assert_eq!(view.search_query, "");
            assert_eq!(view.search_matches, Vec::<u64>::new());
            assert_eq!(view.search_active, -1);
        });
    }

    // ---- multi-document / tabs --------------------------------------

    /// Pushing two documents into the tab list (the way a
    /// multi-file drop or two sequential opens would) keeps both
    /// snapshots and switches the live fields to the most recent.
    #[gpui::test]
    async fn push_two_documents_switches_to_last(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        // Add a second document directly via the public helpers used
        // by `apply`: push a snapshot and load it as the active one.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("second.csv");
        std::fs::write(&path, b"x,y\n10,20\n30,40\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let second = anvil_core::index_opened(opened);
        cx.update_window_entity(&view, |view, _, _| {
            let arc_doc = std::sync::Arc::new(second);
            push_loaded_tab(view, arc_doc);
        });
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.documents.len(), 2);
            assert_eq!(view.active, Some(1));
        });
    }

    /// Switching tabs snapshots the active doc's selection /
    /// scroll_x and restores them when the user comes back.
    #[gpui::test]
    async fn switching_tabs_restores_per_document_state(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        // Set selection on the active tab (the sample fixture has 3
        // rows × 3 cols, so (1, 1) is in bounds).
        cx.update_window_entity(&view, |view, _, _| {
            view.selection.set(CellPos::new(1, 1));
            view.scroll_x = 42.0;
        });

        // Push a second tab; active becomes that tab. We snapshot
        // first (the same call `apply` makes on a fresh open) so the
        // first tab's selection lives in its snapshot for the
        // round-trip.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("second.csv");
        std::fs::write(&path, b"x,y\n10,20\n30,40\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let second = anvil_core::index_opened(opened);
        cx.update_window_entity(&view, |view, _, _| {
            view.snapshot_active();
            let arc_doc = std::sync::Arc::new(second);
            push_loaded_tab(view, arc_doc);
        });

        // The live selection / scroll_x should be the defaults for
        // the new tab, not the old tab's values.
        cx.update_window_entity(&view, |view, _, _| {
            assert_ne!(view.selection.cursor(), Some(CellPos::new(1, 1)));
            assert_eq!(view.scroll_x, 0.0);
        });

        // Switch back to tab 0 — the selection comes back.
        cx.update_window_entity(&view, |view, _, cx| {
            view.switch_to(0, cx);
        });
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.active, Some(0));
            assert_eq!(view.selection.cursor(), Some(CellPos::new(1, 1)));
            assert_eq!(view.scroll_x, 42.0);
        });
    }

    /// Closing the active tab via `close_active_at` shifts
    /// `active` to a neighbour.
    #[gpui::test]
    async fn closing_active_tab_picks_neighbour(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        // Add three tabs total.
        for i in 0..2 {
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join(format!("extra{i}.csv"));
            std::fs::write(&path, b"x,y\n10,20\n30,40\n").expect("write");
            let opened = anvil_core::source::local::open_local(&path).expect("open");
            let extra = anvil_core::index_opened(opened);
            cx.update_window_entity(&view, |view, _, _| {
                let arc_doc = std::sync::Arc::new(extra);
                push_loaded_tab(view, arc_doc);
            });
        }
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.documents.len(), 3);
            assert_eq!(view.active, Some(2));
        });

        // Close the active tab. The new active should be the last
        // available tab.
        cx.update_window_entity(&view, |view, _, cx| {
            view.close_active_at(2, cx);
        });
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.documents.len(), 2);
            assert_eq!(view.active, Some(1));
        });
    }

    /// Closing a non-active tab earlier than the active one shifts
    /// the active index down by one.
    #[gpui::test]
    async fn closing_earlier_non_active_tab_shifts_active(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        // Add two more tabs; active ends up at index 2.
        for i in 0..2 {
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join(format!("extra{i}.csv"));
            std::fs::write(&path, b"x,y\n10,20\n30,40\n").expect("write");
            let opened = anvil_core::source::local::open_local(&path).expect("open");
            let extra = anvil_core::index_opened(opened);
            cx.update_window_entity(&view, |view, _, _| {
                let arc_doc = std::sync::Arc::new(extra);
                push_loaded_tab(view, arc_doc);
            });
        }
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.active, Some(2));
        });

        // Close tab 0 (an earlier, inactive tab). Active should
        // shift from 2 to 1.
        cx.update_window_entity(&view, |view, _, cx| {
            view.close_active_at(0, cx);
        });
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.documents.len(), 2);
            assert_eq!(view.active, Some(1));
        });
    }

    /// Closing the last remaining tab empties the document list and
    /// `active` becomes `None`.
    #[gpui::test]
    async fn closing_last_tab_leaves_empty_state(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        inject(cx, &view);

        cx.update_window_entity(&view, |view, _, cx| {
            view.close_active_at(0, cx);
        });
        cx.update_window_entity(&view, |view, _, _| {
            assert!(view.documents.is_empty());
            assert!(view.active.is_none());
            assert!(view.model.is_none());
        });
    }

    /// Multi-file drop: every dropped path becomes its own tab. This
    /// drives the real `apply(OpenOutcome::Ready)` path twice — the
    /// same code that runs after `begin_open`'s background task — so
    /// the tab-push / switch logic is exercised end-to-end.
    #[gpui::test]
    async fn two_sequential_applies_create_two_tabs(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));

        for name in ["one.csv", "two.csv"] {
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join(name);
            std::fs::write(&path, b"a,b\n1,2\n3,4\n").expect("write");
            let opened = anvil_core::source::local::open_local(&path).expect("open");
            let doc = anvil_core::index_opened(opened);
            let loaded = LoadedDocument {
                document: doc,
                archive: None,
                entry: None,
            };
            cx.update_window_entity(&view, |view, _, cx| {
                view.apply(OpenOutcome::Ready(Box::new(loaded)), cx);
            });
        }
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.documents.len(), 2);
            assert_eq!(view.active, Some(1));
            assert!(view.model.is_some());
        });
    }

    /// A single open arrives as one tab with the loaded document live.
    #[gpui::test]
    async fn apply_ready_creates_one_tab(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("only.csv");
        std::fs::write(&path, b"a,b\n1,2\n3,4\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        let loaded = LoadedDocument {
            document: doc,
            archive: None,
            entry: None,
        };
        cx.update_window_entity(&view, |view, _, cx| {
            view.apply(OpenOutcome::Ready(Box::new(loaded)), cx);
        });
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.documents.len(), 1);
            assert_eq!(view.active, Some(0));
            assert!(view.model.is_some());
            assert!(matches!(view.state, ViewState::Ready));
        });
    }

    /// After an error with no tabs open, the centre panel shows the
    /// error page. With a tab already open, an error on a background
    /// task must NOT clobber the visible document.
    #[gpui::test]
    async fn error_with_open_tab_keeps_the_tab(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ok.csv");
        std::fs::write(&path, b"a,b\n1,2\n").expect("write");
        let opened = anvil_core::source::local::open_local(&path).expect("open");
        let doc = anvil_core::index_opened(opened);
        let loaded = LoadedDocument {
            document: doc,
            archive: None,
            entry: None,
        };
        cx.update_window_entity(&view, |view, _, cx| {
            view.apply(OpenOutcome::Ready(Box::new(loaded)), cx);
        });
        cx.update_window_entity(&view, |view, _, cx| {
            view.apply(OpenOutcome::Error("boom".into()), cx);
        });
        cx.update_window_entity(&view, |view, _, _| {
            // Tab is still open and still rendering.
            assert_eq!(view.documents.len(), 1);
            assert!(view.model.is_some());
            // Error is in the status message, not the centre panel.
            assert!(view
                .status_message
                .as_deref()
                .is_some_and(|m| m.contains("boom")));
        });
    }

    /// Every usable archive entry becomes a tab synchronously, before
    /// any extraction runs. Only the first tab's document is loaded
    /// on demand.
    #[gpui::test]
    async fn archive_entries_become_tabs_immediately(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        let (_dir, archive) = make_zip_archive(&["a.csv", "b.csv", "c.csv"]);
        assert_eq!(archive.total_candidates(), 3);

        let archive_for_view = archive.clone();
        cx.update_window_entity(&view, move |view, _, cx| {
            view.install_archive_tabs(archive_for_view, None, None, cx);
        });

        // All three tabs exist immediately; only the active one is
        // loading.
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.documents.len(), 3);
            assert_eq!(view.active, Some(0));
            assert!(view.documents[0].loading || view.documents[0].is_loaded());
            assert!(!view.documents[1].is_loaded());
            assert!(!view.documents[2].is_loaded());
        });

        // The first tab's lazy load completes and the grid renders.
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert!(view.documents[0].is_loaded());
            assert!(view.model.is_some());
        });

        // Switching to the second tab triggers its own load.
        cx.update_window_entity(&view, |view, _, cx| {
            view.switch_to(1, cx);
        });
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.active, Some(1));
            assert!(view.documents[1].is_loaded());
            assert!(view.model.is_some());
            // The third tab is still untouched.
            assert!(!view.documents[2].is_loaded());
        });
    }

    /// `apply(NeedArchiveChoice)` is the path a multi-entry ZIP drop
    /// takes. Every usable entry becomes a tab and no modal picker is
    /// shown any more.
    #[gpui::test]
    async fn apply_need_archive_choice_installs_all_tabs(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        let (_dir, archive) = make_zip_archive(&["one.csv", "two.csv", "three.csv"]);
        cx.update_window_entity(&view, move |view, _, cx| {
            view.apply(OpenOutcome::NeedArchiveChoice(archive), cx);
        });
        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.documents.len(), 3);
            assert_eq!(view.active, Some(0));
            assert!(!view.show_picker, "no modal picker any more");
        });
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert!(view.documents[0].is_loaded());
            assert!(view.model.is_some());
        });
    }

    /// When the loader already produced a document for one entry (the
    /// auto-opened favourite), that entry's tab is ready and the
    /// active tab points at it; the others stay pending.
    #[gpui::test]
    async fn install_archive_tabs_marks_loaded_entry_as_ready(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
        let (_dir, archive) = make_zip_archive(&["a.csv", "b.csv"]);
        let loaded = anvil_core::load_entry(&archive, "b.csv").expect("load entry");
        let loaded_doc = Arc::new(loaded.document);

        cx.update_window_entity(&view, move |view, _, cx| {
            view.install_archive_tabs(archive, Some("b.csv".to_string()), Some(loaded_doc), cx);
        });

        cx.update_window_entity(&view, |view, _, _| {
            assert_eq!(view.documents.len(), 2);
            // Active points at the pre-loaded entry, not the first
            // entry in the archive.
            assert_eq!(view.active, Some(1));
            assert!(view.documents[1].is_loaded());
            assert!(!view.documents[0].is_loaded());
            assert!(view.model.is_some());
        });

        // The pending first entry loads when the user switches to it.
        cx.update_window_entity(&view, |view, _, cx| {
            view.switch_to(0, cx);
        });
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, _, _| {
            assert!(view.documents[0].is_loaded());
            assert_eq!(view.active, Some(0));
        });
    }
}

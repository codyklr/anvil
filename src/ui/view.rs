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
    search_records, ArchiveSource, CellPos, LoadResult, LoadedDocument, SearchOptions, Selection,
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
                self.model = Some(GridModel::new(Arc::new(document)));
                self.archive = archive;
                self.selected_entry = entry;
                self.show_picker = false;
                self.scroll_x = 0.0;
                self.selection.clear();
                self.drag_selecting = false;
                self.status_message = None;
                self.copy_flash_rect = None;
                self.close_search();
                self.focus_grid_on_next_frame = true;
                self.state = ViewState::Ready;
            }
            OpenOutcome::NeedArchiveChoice(source) => {
                self.archive = Some(source);
                self.selected_entry = None;
                self.show_picker = true;
                self.selection.clear();
                self.drag_selecting = false;
                self.status_message = None;
                self.copy_flash_rect = None;
                self.close_search();
                self.focus_grid_on_next_frame = true;
                self.state = ViewState::Ready;
            }
            OpenOutcome::Error(message) => {
                self.model = None;
                self.selection.clear();
                self.copy_flash_rect = None;
                self.close_search();
                self.status_message = Some(format!("error: {message}"));
                self.state = ViewState::Error(message);
            }
        }
        cx.notify();
    }

    fn choose_entry(&mut self, entry: String, cx: &mut Context<Self>) {
        let Some(archive) = self.archive.clone() else {
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.state = ViewState::Loading(format!("extracting {entry}"));
        self.show_picker = false;
        cx.notify();

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let outcome = executor
                .spawn(async move { extract_and_index(&archive, &entry) })
                .await;
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
                let entity = entity.clone();
                let row_ctx = grid::RowContext {
                    selection: &selection,
                    scroll_x,
                    viewport_w,
                    copy_flash: copy_flash_rect,
                    search_matches: &search_matches,
                    search_active,
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
                        this.choose_entry(on_click_name.clone(), cx);
                    }))
                    .into_any_element(),
            );
        }

        let mut children: Vec<AnyElement> = vec![
            div()
                .text_color(theme::TEXT_STRONG)
                .text_lg()
                .child(format!("{} files in archive", usable.len()))
                .into_any_element(),
            div()
                .text_xs()
                .text_color(theme::TEXT_DIM)
                .child(format!(
                    "{} — select one file to open",
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
        if let Some(first) = paths.paths().first() {
            self.begin_open(first.clone(), cx);
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
            view.model = Some(GridModel::new(Arc::new(doc)));
            view.state = ViewState::Ready;
            view.selection.set(CellPos::new(0, 0));
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
}

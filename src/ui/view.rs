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
    canvas, div, prelude::*, px, rgb, uniform_list, AnyElement, Context, ExternalPaths,
    FocusHandle, KeyDownEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, ScrollDelta,
    ScrollWheelEvent, Task, UniformListScrollHandle, Window,
};

use anvil_core::{ArchiveSource, CellPos, LoadResult, LoadedDocument, Selection};

use super::archive_picker;
use super::grid::{self, GridModel};
use super::theme;

/// How many rows a Page Up / Page Down jump moves the cursor by. Tied
/// to a conservative estimate of the visible row count so the grid does
/// not have to be measured every keypress.
const PAGE_STEP: i64 = 20;

/// How long the green "copied" flash stays over the selection before it
/// fades back to the normal selection colour.
const COPY_FLASH_DURATION: Duration = Duration::from_millis(450);

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
            task: None,
            status_message: None,
            focus_handle: cx.focus_handle(),
            focus_grid_on_next_frame: false,
            clipboard: Arc::new(system_clipboard_write),
            copy_flash_rect: None,
            copy_flash_generation: 0,
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
                self.status_message = None;
                self.copy_flash_rect = None;
                self.focus_grid_on_next_frame = true;
                self.state = ViewState::Ready;
            }
            OpenOutcome::NeedArchiveChoice(source) => {
                self.archive = Some(source);
                self.selected_entry = None;
                self.show_picker = true;
                self.selection.clear();
                self.status_message = None;
                self.copy_flash_rect = None;
                self.focus_grid_on_next_frame = true;
                self.state = ViewState::Ready;
            }
            OpenOutcome::Error(message) => {
                self.model = None;
                self.selection.clear();
                self.copy_flash_rect = None;
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
                let entity = entity.clone();
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
                            scroll_x,
                            viewport_w,
                            &selection,
                            copy_flash_rect,
                            move |ev, row, col, app_cx| {
                                let pos = CellPos::new(row, col);
                                let extend = ev.modifiers.shift;
                                entity_for_row.update(app_cx, |view, cx| {
                                    if extend {
                                        view.selection.extend_to(pos);
                                    } else {
                                        view.selection.set(pos);
                                    }
                                    cx.notify();
                                });
                            },
                        )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .size_full()
        .track_scroll(self.v_scroll.clone());

        div()
            .flex()
            .flex_col()
            .flex_1()
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .key_context("Grid")
                    // Keyboard events are only routed to `on_key_down`
                    // while this element holds focus. `track_focus`
                    // also makes a mouse-down anywhere in the grid
                    // transfer focus, so clicking a cell then pressing
                    // Cmd/Ctrl+C works.
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::on_grid_key_down))
                    .on_scroll_wheel(cx.listener(Self::on_grid_scroll))
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
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let shift = modifiers.shift;

        // Cmd/Ctrl+C: copy the selection to the clipboard. The
        // "secondary" modifier is Cmd on macOS and Ctrl elsewhere.
        if key == "c" && modifiers.secondary() && !modifiers.alt && !modifiers.function {
            self.copy_selection_to_clipboard(cx);
            return;
        }

        // Movement deltas in (row, col). `None` means the key is not a
        // movement key; we then check for "clear selection" / "select
        // all".
        let movement: Option<(i64, i64)> = match key {
            "up" => Some((-1, 0)),
            "down" => Some((1, 0)),
            "left" => Some((0, -1)),
            "right" => Some((0, 1)),
            "pageup" => Some((-PAGE_STEP, 0)),
            "pagedown" => Some((PAGE_STEP, 0)),
            "home" => Some((0, i64::MIN)),
            "end" => Some((0, i64::MAX)),
            _ => None,
        };

        if let Some((dr, dc)) = movement {
            self.move_selection(dr, dc, shift, cx);
            return;
        }

        if key == "escape" {
            if !self.selection.is_empty() {
                self.selection.clear();
                cx.notify();
            } else if self.show_picker {
                self.show_picker = false;
                cx.notify();
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

    fn render_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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
        let center = self.render_center(window, cx);
        let overlay = self.render_overlay(cx);

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
                    .child(center)
                    .when_some(overlay, |this, overlay| this.child(overlay)),
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

    /// Inject a ready document and focus the grid the way `render_grid`
    /// does after an open. Returns a recorder standing in for the system
    /// clipboard so the copy path can be asserted without side effects.
    fn inject(cx: &mut VisualTestContext, view: &Entity<MainView>) -> Arc<Mutex<Vec<String>>> {
        let written: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = written.clone();
        cx.update_window_entity(view, |view, window, cx| {
            view.model = Some(GridModel::new(Arc::new(sample_document())));
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
}

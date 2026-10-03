//! Virtualized grid, column resize and horizontal scrollbar.
//!
//! Extracted verbatim from `view.rs` (behaviour-neutral split).

use super::*;
use gpui_kit::base::{Scrollbar, ScrollbarMode};
use gpui_kit::component::ActiveTheme;

/// Width of the soft fade painted at a horizontal scroll boundary.
const EDGE_FADE_WIDTH: f32 = 28.;

impl MainView {
    pub(super) fn render_grid(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(model) = self.model.clone() else {
            return div().into_any_element();
        };
        let colors = theme::palette(cx);

        // A document just became ready: move keyboard focus onto the
        // grid so arrow navigation and Cmd/Ctrl+C work without a click.
        // Focus is moved on the next frame rather than mid-render.
        if self.focus_grid_on_next_frame {
            self.focus_grid_on_next_frame = false;
            let focus_handle = self.focus_handle.clone();
            cx.defer_in(window, move |_view, window, cx| {
                window.focus(&focus_handle, cx);
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
        let header_ctx = grid::HeaderContext {
            scroll_x,
            viewport_w,
            scoped_column: self.search_column,
            sort: sort_indicator,
            colors: theme::palette(cx),
        };
        let header = grid::render_header(
            &model,
            &header_ctx,
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
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                let colors = theme::palette(cx);
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
                    colors,
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
        .font_family(cx.theme().mono_font_family.clone())
        .track_scroll(&self.v_scroll);

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

        // Fade the column edge where the viewport cuts the data. Without it
        // a half-drawn column reads as clipped text; with it, the same edge
        // reads as "there is more to the right". Only drawn when there is
        // actually more to reach.
        let more_right = scroll_x + viewport_w < model.layout.total - 0.5;
        let more_left = scroll_x > 0.5;
        let edge = texture::edge_fade(colors.surface, true);
        let edge_left = texture::edge_fade(colors.surface, false);

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .overflow_hidden()
            .bg(colors.surface)
            .font_family(cx.theme().font_family.clone())
            .text_color(cx.theme().foreground)
            .cursor_default()
            .child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .bg(colors.surface)
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
                        gpui_kit::MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.drag_selecting = false;
                            view.stop_auto_scroll();
                            view.kick_inspector(cx);
                        }),
                    )
                    .on_mouse_up_out(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.drag_selecting = false;
                            view.stop_auto_scroll();
                            view.kick_inspector(cx);
                        }),
                    )
                    .child(auto_scroll_overlay)
                    .child(list)
                    .when(more_left, |viewport| {
                        viewport.child(
                            texture::scrim(edge_left)
                                .left(px(0.))
                                .top(px(0.))
                                .bottom(px(0.))
                                .w(px(EDGE_FADE_WIDTH))
                                .bg(edge_left),
                        )
                    })
                    .when(more_right, |viewport| {
                        viewport.child(
                            texture::scrim(edge)
                                .right(px(0.))
                                .top(px(0.))
                                .bottom(px(0.))
                                .w(px(EDGE_FADE_WIDTH))
                                .bg(edge),
                        )
                    })
                    .child(Scrollbar::vertical(&self.v_scroll).mode(ScrollbarMode::Always)),
            )
            .child(self.render_h_scrollbar(cx))
            .when(self.focus_handle.is_focused(window), |grid| {
                // An overlay keeps focus decoration out of viewport geometry.
                grid.child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .left(px(0.))
                        .size_full()
                        .border_1()
                        .border_color(colors.focus_ring),
                )
            })
            .into_any_element()
    }

    // ---- column resize ---------------------------------------------------

    /// Begin a column-resize drag from a header boundary. `pointer_x` is
    /// the press position and `width` the column's width at press time.
    pub(super) fn start_column_drag(
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
    pub(super) fn column_drag_move(&mut self, pointer_x: f32, cx: &mut Context<Self>) {
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
    pub(super) fn end_column_drag(&mut self, cx: &mut Context<Self>) {
        if self.column_drag.take().is_some() {
            self.snapshot_active();
            cx.notify();
        }
    }

    /// The horizontal scrollbar.
    ///
    /// It overlays the bottom edge of the grid rather than taking a row of
    /// its own: the data viewport is the scarce resource, and a 12 px band
    /// below it is chrome the user did not ask for. The thumb is thin and
    /// low-contrast until it is under the pointer, so it stays out of the
    /// way of the last visible row.
    pub(super) fn render_h_scrollbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme::palette(cx);
        let (viewport, total) = match &self.model {
            Some(model) => (self.viewport_w, model.layout.total),
            None => (0.0, 0.0),
        };
        if total <= viewport || viewport <= 0.0 {
            return div().h(px(0.)).into_any_element();
        }
        let frac = (viewport / total).clamp(0.05, 1.0);
        let thumb_w = frac * viewport;
        let max_scroll = (total - viewport).max(1.0);
        let thumb_left = (self.scroll_x / max_scroll).clamp(0.0, 1.0) * (viewport - thumb_w);
        let entity = cx.entity();
        let thumb_bg = theme::blend(colors.surface, colors.muted_foreground, 0.42);
        let thumb_bg_hover = theme::blend(colors.surface, colors.muted_foreground, 0.72);

        div()
            .id("anvil-h-scroll")
            .relative()
            .h(px(12.))
            .w_full()
            .flex_shrink_0()
            .border_t_1()
            .border_color(colors.row_border)
            .bg(colors.surface)
            .child(
                div()
                    .absolute()
                    .top(px(4.))
                    .left(px(thumb_left))
                    .h(px(4.))
                    .w(px(thumb_w))
                    .rounded_full()
                    .bg(thumb_bg)
                    .hover(|style| style.bg(thumb_bg_hover))
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
            .into_any_element()
    }

    pub(super) fn on_grid_scroll(
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

    pub(super) fn scroll_horizontal(&mut self, delta: f32, cx: &mut Context<Self>) {
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
    pub(super) fn ensure_column_visible(&mut self, column: usize, cx: &mut Context<Self>) {
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
}

//! Resizable workspace geometry. The shell and tools retain their own content;
//! this module owns only pane widths, visibility, and the shared grid viewport.

use gpui_kit::{
    component::resizable::{h_resizable, resizable_panel, ResizableState},
    div,
    prelude::*,
    px, AnyElement, Context, Entity, Subscription, Window,
};

use super::{theme, MainView};

const MIN_PRIMARY: f32 = 320.;
const MIN_SIDEBAR: f32 = 140.;
const MIN_AUXILIARY: f32 = 200.;
const MAX_SIDEBAR: f32 = 360.;
const MAX_AUXILIARY: f32 = 560.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Auxiliary {
    Inspector,
    Diagnostics,
    CellDetail,
}

impl Auxiliary {
    fn index(self) -> usize {
        match self {
            Self::Inspector => 0,
            Self::Diagnostics => 1,
            Self::CellDetail => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    width: f32,
    sidebar: f32,
    auxiliary: f32,
    sidebar_min: f32,
    sidebar_max: f32,
    auxiliary_min: f32,
    auxiliary_max: f32,
    primary_min: f32,
}

impl Geometry {
    fn new(width: f32, sidebar: Option<f32>, auxiliary: Option<f32>) -> Self {
        let width = width.max(0.);
        let primary_min = width.min(MIN_PRIMARY);
        let budget = width - primary_min;
        let minimum = sidebar.map_or(0., |_| MIN_SIDEBAR) + auxiliary.map_or(0., |_| MIN_AUXILIARY);
        let scale = if minimum > 0. {
            (budget / minimum).min(1.)
        } else {
            1.
        };
        let sidebar_min = sidebar.map_or(0., |_| MIN_SIDEBAR * scale);
        let auxiliary_min = auxiliary.map_or(0., |_| MIN_AUXILIARY * scale);
        let sidebar_max = sidebar.map_or(0., |_| {
            MAX_SIDEBAR.min((budget - auxiliary_min).max(sidebar_min))
        });
        let auxiliary_max = auxiliary.map_or(0., |_| {
            MAX_AUXILIARY.min((budget - sidebar_min).max(auxiliary_min))
        });
        let mut sidebar = sidebar.map_or(0., |w| w.clamp(sidebar_min, sidebar_max));
        let mut auxiliary = auxiliary.map_or(0., |w| w.clamp(auxiliary_min, auxiliary_max));
        if sidebar + auxiliary > budget {
            let spare = (budget - sidebar_min - auxiliary_min).max(0.);
            let desired = sidebar - sidebar_min + auxiliary - auxiliary_min;
            let scale = if desired > 0. { spare / desired } else { 0. };
            sidebar = sidebar_min + (sidebar - sidebar_min) * scale;
            auxiliary = auxiliary_min + (auxiliary - auxiliary_min) * scale;
        }
        Self {
            width,
            sidebar,
            auxiliary,
            sidebar_min,
            sidebar_max,
            auxiliary_min,
            auxiliary_max,
            primary_min,
        }
    }

    fn primary(self) -> f32 {
        (self.width - self.sidebar - self.auxiliary).max(0.)
    }

    fn viewport(self) -> f32 {
        (self.primary() - f32::from(theme::ROW_NUMBER_WIDTH)).max(0.)
    }
}

pub(super) struct WorkspaceContents {
    pub body: AnyElement,
    pub panel: Option<AnyElement>,
    pub inspector: Option<AnyElement>,
    pub diagnostics: Option<AnyElement>,
    pub cell_detail: Option<AnyElement>,
}

pub(super) struct Workspace {
    sidebar_width: f32,
    auxiliary_widths: [f32; 3],
    previous_tools: [bool; 3],
    selected: Option<Auxiliary>,
    signature: Option<(f32, bool, Option<Auxiliary>)>,
    geometry: Option<Geometry>,
    state: Option<Entity<ResizableState>>,
    observation: Option<Subscription>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            sidebar_width: f32::from(theme::FILE_PANEL_WIDTH),
            auxiliary_widths: [
                f32::from(theme::INSPECTOR_WIDTH),
                f32::from(theme::DIAGNOSTICS_WIDTH),
                f32::from(theme::CELL_DETAIL_WIDTH),
            ],
            previous_tools: [false; 3],
            selected: None,
            signature: None,
            geometry: None,
            state: None,
            observation: None,
        }
    }
}

impl Workspace {
    // Open handlers normally make these mutually exclusive. If multiple flags
    // survive, the newly activated tool wins; ties prefer the full cell value.
    fn select(&self, tools: [bool; 3]) -> Option<Auxiliary> {
        let order = [
            Auxiliary::CellDetail,
            Auxiliary::Diagnostics,
            Auxiliary::Inspector,
        ];
        order
            .into_iter()
            .find(|tool| tools[tool.index()] && !self.previous_tools[tool.index()])
            .or_else(|| self.selected.filter(|tool| tools[tool.index()]))
            .or_else(|| order.into_iter().find(|tool| tools[tool.index()]))
    }

    fn scroll_after_resize(&self, viewport: f32, total: f32, scroll: f32) -> f32 {
        // A document snapshot owns its scroll position. Only a change to
        // workspace geometry should constrain it, not an ordinary tab render.
        if self
            .geometry
            .is_some_and(|previous| (previous.viewport() - viewport).abs() > 0.5)
        {
            scroll.clamp(0., (total - viewport).max(0.))
        } else {
            scroll
        }
    }

    fn geometry(&self, width: f32, sidebar: bool, selected: Option<Auxiliary>) -> Geometry {
        Geometry::new(
            width,
            sidebar.then_some(self.sidebar_width),
            selected.map(|tool| self.auxiliary_widths[tool.index()]),
        )
    }
}

impl MainView {
    pub(super) fn workspace_grid_viewport_width(&mut self, window: &Window) -> f32 {
        let width = self.workspace_viewport_width(window);
        if let Some(model) = &self.model {
            self.scroll_x =
                self.workspace
                    .scroll_after_resize(width, model.layout.total, self.scroll_x);
        }
        width
    }

    pub(super) fn workspace_viewport_width(&self, window: &Window) -> f32 {
        let tools = [
            self.inspector_open && self.model.is_some(),
            self.diagnostics_open && self.model.is_some(),
            self.cell_detail_open && self.model.is_some(),
        ];
        self.workspace
            .geometry(
                f32::from(window.viewport_size().width),
                self.file_panel_open && !self.documents.is_empty(),
                self.workspace.select(tools),
            )
            .viewport()
    }

    pub(super) fn render_workspace(
        &mut self,
        contents: WorkspaceContents,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let WorkspaceContents {
            body,
            panel,
            inspector,
            diagnostics,
            cell_detail,
        } = contents;
        let tools = [
            inspector.is_some(),
            diagnostics.is_some(),
            cell_detail.is_some(),
        ];
        let selected = self.workspace.select(tools);
        let sidebar = panel.is_some();
        let width = f32::from(window.viewport_size().width);
        let geometry = self.workspace.geometry(width, sidebar, selected);
        let signature = (width, sidebar, selected);
        self.workspace.previous_tools = tools;
        self.workspace.selected = selected;
        self.workspace.geometry = Some(geometry);

        // Slot indices change with visibility, and kit state proportionally
        // scales on container resize. Start fresh in those cases, seeding the
        // slots from retained preferences instead of stale measured bounds.
        if self.workspace.signature != Some(signature) {
            self.workspace.observation = None;
            let state = cx.new(|_| ResizableState::default());
            self.workspace.observation = Some(cx.observe(&state, |view, state, cx| {
                let Some((width, sidebar, selected)) = view.workspace.signature else {
                    return;
                };
                let Some(geometry) = view.workspace.geometry else {
                    return;
                };
                let state = state.read(cx);
                let sizes = state.sizes();
                if (f32::from(state.container_size()) - width).abs() > 0.5
                    || sizes.len() != 1 + usize::from(sidebar) + usize::from(selected.is_some())
                {
                    return;
                }
                let mut changed = false;
                if sidebar {
                    let size =
                        f32::from(sizes[0]).clamp(geometry.sidebar_min, geometry.sidebar_max);
                    if (size - geometry.sidebar).abs() > 0.5 {
                        view.workspace.sidebar_width = size;
                        changed = true;
                    }
                }
                if let Some(tool) = selected {
                    let size = f32::from(sizes[sizes.len() - 1])
                        .clamp(geometry.auxiliary_min, geometry.auxiliary_max);
                    if (size - geometry.auxiliary).abs() > 0.5 {
                        view.workspace.auxiliary_widths[tool.index()] = size;
                        changed = true;
                    }
                }
                // Kit notifies during every prepaint as well as every drag.
                // Only an actual width change should invalidate the grid.
                if changed {
                    cx.notify();
                }
            }));
            self.workspace.state = Some(state);
            self.workspace.signature = Some(signature);
        }

        let auxiliary = match selected {
            Some(Auxiliary::Inspector) => inspector,
            Some(Auxiliary::Diagnostics) => diagnostics,
            Some(Auxiliary::CellDetail) => cell_detail,
            None => None,
        };
        let state = self
            .workspace
            .state
            .as_ref()
            .expect("workspace state initialized");
        let mut group = h_resizable("anvil-workspace").with_state(state);
        if let Some(panel) = panel {
            group = group.child(
                resizable_panel()
                    .size(px(geometry.sidebar))
                    .flex_none()
                    .size_range(px(geometry.sidebar_min)..px(geometry.sidebar_max))
                    .child(div().size_full().min_w_0().overflow_hidden().child(panel)),
            );
        }
        group = group.child(
            resizable_panel()
                .size_range(px(geometry.primary_min)..px(width))
                .child(
                    div()
                        .size_full()
                        .flex()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(body),
                ),
        );
        if let Some(auxiliary) = auxiliary {
            group = group.child(
                resizable_panel()
                    .size(px(geometry.auxiliary))
                    .flex_none()
                    .size_range(px(geometry.auxiliary_min)..px(geometry.auxiliary_max))
                    .child(
                        div()
                            .size_full()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(auxiliary),
                    ),
            );
        }
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(group)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_is_clamped_only_when_workspace_width_changes() {
        let geometry = Geometry::new(1000., Some(220.), None);
        let workspace = Workspace {
            geometry: Some(geometry),
            ..Workspace::default()
        };
        let viewport = geometry.viewport();
        assert_eq!(workspace.scroll_after_resize(viewport, 100., 42.), 42.);
        assert_eq!(
            workspace.scroll_after_resize(viewport + 100., 100., 42.),
            0.
        );
        assert_eq!(
            workspace.scroll_after_resize(viewport + 100., 1000., 400.),
            1000. - viewport - 100.
        );
        assert_eq!(
            Workspace::default().scroll_after_resize(viewport, 100., 42.),
            42.
        );
    }

    #[test]
    fn default_widths_and_unobstructed_viewport() {
        let workspace = Workspace::default();
        for width in [200., 800., 1440., 2400.] {
            assert_eq!(
                workspace.geometry(width, false, None).viewport(),
                (width - f32::from(theme::ROW_NUMBER_WIDTH)).max(0.)
            );
        }
        assert_eq!(
            workspace
                .geometry(1440., true, Some(Auxiliary::Inspector))
                .viewport(),
            1440. - 220. - 320. - f32::from(theme::ROW_NUMBER_WIDTH)
        );
        assert_eq!(
            workspace
                .geometry(1440., true, Some(Auxiliary::CellDetail))
                .auxiliary,
            360.
        );
    }

    #[test]
    fn narrow_normal_and_wide_preserve_primary_and_cap_tools() {
        for width in [0., 200., 320., 480., 660., 800., 1440., 2400.] {
            for sidebar in [None, Some(220.), Some(10000.)] {
                for auxiliary in [None, Some(320.), Some(10000.)] {
                    let geometry = Geometry::new(width, sidebar, auxiliary);
                    assert!(geometry.primary() + 0.001 >= width.min(MIN_PRIMARY));
                    assert!(geometry.sidebar <= MAX_SIDEBAR);
                    assert!(geometry.auxiliary <= MAX_AUXILIARY);
                    assert!(
                        (geometry.primary() + geometry.sidebar + geometry.auxiliary - width).abs()
                            < 0.001
                    );
                }
            }
        }
    }

    #[test]
    fn last_activated_tool_uses_one_surface() {
        let mut workspace = Workspace::default();
        assert_eq!(
            workspace.select([true, false, false]),
            Some(Auxiliary::Inspector)
        );
        workspace.previous_tools = [true, false, false];
        workspace.selected = Some(Auxiliary::Inspector);
        assert_eq!(
            workspace.select([true, true, false]),
            Some(Auxiliary::Diagnostics)
        );
        workspace.previous_tools = [true, true, false];
        workspace.selected = Some(Auxiliary::Diagnostics);
        assert_eq!(
            workspace.select([true, true, true]),
            Some(Auxiliary::CellDetail)
        );
        workspace.previous_tools = [true; 3];
        workspace.selected = Some(Auxiliary::CellDetail);
        assert_eq!(workspace.select([true; 3]), Some(Auxiliary::CellDetail));
        assert_eq!(workspace.select([false; 3]), None);
    }

    #[gpui_kit::test]
    async fn kit_resize_and_tool_switch_match_grid_bounds(cx: &mut gpui_kit::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            gpui_kit::init(cx);
            theme::init(cx);
            MainView::new(cx)
        });
        cx.simulate_resize(gpui_kit::size(px(1440.), px(800.)));
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("workspace.csv");
        let headers = (0..30)
            .map(|column| format!("column_{column}"))
            .collect::<Vec<_>>()
            .join(",");
        let values = vec!["value"; 30].join(",");
        std::fs::write(&path, format!("{headers}\n{values}\n{values}\n")).expect("write fixture");
        let opened = anvil_core::source::local::open_local(&path).expect("open fixture");
        let document = std::sync::Arc::new(anvil_core::index_opened(opened));
        cx.update_window_entity(&view, |view, _, cx| {
            let id = view.alloc_doc_id();
            view.documents
                .push(super::super::DocumentSnapshot::from_doc(
                    id, document, None, None,
                ));
            view.active = Some(0);
            view.load_active();
            view.inspector_open = true;
            cx.notify();
        });
        cx.run_until_parked();
        let assert_measured = |view: &MainView, window: &Window| {
            let bounds = view.grid_bounds.expect("grid bounds");
            let measured =
                (f32::from(bounds.size.width) - f32::from(theme::ROW_NUMBER_WIDTH)).max(0.);
            assert!(
                (view.viewport_w - measured).abs() < 1.,
                "viewport {} != measured {measured}",
                view.viewport_w
            );
            assert!((view.viewport_w - view.workspace_viewport_width(window)).abs() < 1.);
            let total = view.model.as_ref().expect("grid model").layout.total;
            assert!(view.scroll_x <= (total - view.viewport_w).max(0.));
        };
        cx.update_window_entity(&view, |view, window, cx| {
            assert_measured(view, window);
            let state = view.workspace.state.as_ref().expect("kit state").clone();
            state.update(cx, |state, cx| state.resize_panel(0, px(280.), window, cx));
        });
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, window, cx| {
            assert_measured(view, window);
            assert!((view.workspace.sidebar_width - 280.).abs() < 1.);
            view.scroll_horizontal(f32::MAX, cx);
            let total = view.model.as_ref().expect("grid model").layout.total;
            assert_eq!(view.scroll_x, (total - view.viewport_w).max(0.));
            let state = view.workspace.state.as_ref().expect("kit state").clone();
            state.update(cx, |state, cx| state.resize_panel(2, px(420.), window, cx));
        });
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, window, _| {
            assert_measured(view, window);
            assert!((view.workspace.auxiliary_widths[0] - 420.).abs() < 1.);
        });
        for (width, height) in [(600., 500.), (1000., 700.), (1800., 900.), (1000., 200.)] {
            cx.simulate_resize(gpui_kit::size(px(width), px(height)));
            // Validate the first resize frame, not only the settling frame.
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update_window_entity(&view, |view, window, _| assert_measured(view, window));
            cx.run_until_parked();
            cx.update_window_entity(&view, |view, window, _| assert_measured(view, window));
        }
        cx.update_window_entity(&view, |view, _, cx| {
            view.diagnostics_open = true;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, window, cx| {
            assert_measured(view, window);
            assert_eq!(view.workspace.selected, Some(Auxiliary::Diagnostics));
            view.cell_detail_open = true;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, window, cx| {
            assert_measured(view, window);
            assert_eq!(view.workspace.selected, Some(Auxiliary::CellDetail));
            view.inspector_open = false;
            view.diagnostics_open = false;
            view.cell_detail_open = false;
            view.file_panel_open = false;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update_window_entity(&view, |view, window, _| {
            assert_measured(view, window);
            assert_eq!(
                view.viewport_w,
                f32::from(window.viewport_size().width) - f32::from(theme::ROW_NUMBER_WIDTH)
            );
        });
    }

    #[test]
    fn constrained_windows_do_not_destroy_retained_preferences() {
        let workspace = Workspace {
            sidebar_width: 280.,
            auxiliary_widths: [
                420.,
                f32::from(theme::DIAGNOSTICS_WIDTH),
                f32::from(theme::CELL_DETAIL_WIDTH),
            ],
            ..Workspace::default()
        };
        let narrow = workspace.geometry(600., true, Some(Auxiliary::Inspector));
        assert!(narrow.sidebar < 280. && narrow.auxiliary < 420.);
        let wide = workspace.geometry(1600., true, Some(Auxiliary::Inspector));
        assert_eq!((wide.sidebar, wide.auxiliary), (280., 420.));
        assert_eq!(workspace.geometry(1600., false, None).primary(), 1600.);
        assert_eq!(
            workspace.geometry(1600., true, Some(Auxiliary::Inspector)),
            wide
        );
    }
}

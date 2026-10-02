//! GPUI application entry point.

use gpui_kit::{prelude::*, px, size, App, Bounds, TitlebarOptions, WindowBounds, WindowOptions};

use super::view::MainView;
use gpui_kit::component::TitleBar;

const DEFAULT_TITLE: &str = "Anvil";

pub fn run() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            super::theme::init(cx);
            let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some(DEFAULT_TITLE.into()),
                        ..TitleBar::title_bar_options()
                    }),
                    window_min_size: Some(size(px(900.), px(480.))),
                    ..TitleBar::window_options()
                },
                cx,
                |window, cx| {
                    let view = cx.new(MainView::new);
                    view.update(cx, |view, cx| view.startup(window, cx));
                    view
                },
            )
            .expect("could not open main window");
            cx.activate(true);
        });
}

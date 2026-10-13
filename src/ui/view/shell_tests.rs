//! Production shell interactions, hosted by Kit's real window Root.
//!
//! Fixtures enter through `apply(Ready)`; commands are exercised through
//! native pointer/key dispatch, never by invoking their private handlers.

use super::*;
use gpui_kit::base::Root;
use gpui_kit::component::{ActiveTheme, ThemeMode};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AppContext, Entity, TestAppContext, WindowBounds, WindowHandle, WindowOptions};
use std::io::Write;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(5);

// Match gpui-kit 0.7.0 tests/common::open_window: retain the content Entity,
// rather than downcasting the window root to MainView or bypassing Kit init.
fn open_shell(
    cx: &mut TestAppContext,
    loaded: Box<LoadedDocument>,
) -> (WindowHandle<Root>, Entity<MainView>) {
    open_shell_at(cx, Some(loaded), 1200., 800.)
}

fn open_shell_at(
    cx: &mut TestAppContext,
    loaded: Option<Box<LoadedDocument>>,
    width: f32,
    height: f32,
) -> (WindowHandle<Root>, Entity<MainView>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        let (window, content) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds::new(
                    Default::default(),
                    gpui_kit::size(px(width), px(height)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut view = MainView::new(cx);
                    view.focus_handle.focus(window, cx);
                    if let Some(loaded) = loaded {
                        view.apply(OpenOutcome::Ready(loaded), cx);
                    }
                    view
                })
            },
        )
        .expect("open production Kit window");
        (window.downcast::<Root>().expect("Kit Root"), content)
    })
}

fn assert_empty_home(view: &MainView) {
    assert!(matches!(view.state, ViewState::Empty));
    assert!(view.model.is_none());
    assert!(view.documents.is_empty());
    assert!(view.active.is_none());
    assert!(!view.show_picker);
}

fn assert_home_geometry(window: &gpui_kit::Window) {
    let open = window.find("welcome-open");
    assert!(open.visible());
    assert_eq!(open.label(), Some("Open file"));
    let bounds = open.bounds();
    assert!(bounds.size.width >= px(120.));
    assert!(bounds.size.height >= px(32.));
    assert!(bounds.origin.x >= px(0.));
    assert!(bounds.origin.y >= px(0.));
    let viewport = window.viewport_size();
    assert!(bounds.origin.x + bounds.size.width <= viewport.width);
    assert!(bounds.origin.y + bounds.size.height <= viewport.height);
    assert!(window.try_find("welcome-open-dir").is_none());
}

#[gpui_kit::test]
async fn fresh_home_renders_named_open_within_both_window_sizes_and_themes(
    cx: &mut TestAppContext,
) {
    for (width, height) in [(1200., 800.), (900., 480.)] {
        let (handle, view) = open_shell_at(cx, None, width, height);
        wait_for_grid(cx, handle, &view).await;
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            cx.update_window(handle.into(), |_, window, cx| {
                if cx.theme().mode != mode {
                    window.click("appearance", cx);
                }
                window.render_frame(cx);
                assert_eq!(cx.theme().mode, mode);
                assert_home_geometry(window);
                assert_empty_home(view.read(cx));
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
async fn populated_theme_switch_keeps_grid_and_document_at_both_window_sizes(
    cx: &mut TestAppContext,
) {
    for (width, height) in [(1200., 800.), (900., 480.)] {
        let (_fixture, loaded) = csv_fixture();
        let (handle, view) = open_shell_at(cx, Some(loaded), width, height);
        wait_for_grid(cx, handle, &view).await;
        let original = cx.update(|cx| view.read(cx).model.as_ref().unwrap().doc.clone());
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.click("appearance", cx);
                window.render_frame(cx);
                assert_eq!(cx.theme().mode, mode);
                assert!(window.try_find("welcome-open").is_none());
                let state = view.read(cx);
                assert!(matches!(state.state, ViewState::Ready));
                assert_eq!(state.documents.len(), 1);
                assert_shared_active(state);
                assert!(Arc::ptr_eq(&original, &state.model.as_ref().unwrap().doc));
                let bounds = state.grid_bounds.expect("rendered grid bounds");
                assert!(bounds.size.width > px(0.));
                assert!(bounds.size.height > px(0.));
            })
            .unwrap();
        }
    }
}

fn csv_fixture() -> (tempfile::TempDir, Box<LoadedDocument>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("shell.csv");
    std::fs::write(&path, b"name,value\nalpha,007\nbeta,1.10\n").expect("write CSV");
    let anvil_core::loader::LoadResult::Ready(loaded) =
        anvil_core::loader::load(&path).expect("load CSV")
    else {
        panic!("direct CSV must be ready");
    };
    (dir, loaded)
}

#[gpui_kit::test]
async fn filter_tab_accepts_selected_suggestion_before_root_focus_traversal(
    cx: &mut TestAppContext,
) {
    let (_fixture, loaded) = csv_fixture();
    let (handle, view) = open_shell(cx, loaded);
    wait_for_grid(cx, handle, &view).await;
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("filter", cx);
    })
    .unwrap();
    cx.wait_for(handle.into(), WAIT, |window, cx| {
        view.read(cx).filter_focus_handle.is_focused(window)
    })
    .await;
    cx.update_window(handle.into(), |_, window, cx| {
        window.input("name ", cx);
        window.press("down", cx);
        let state = view.read(cx);
        assert!(state.filter_popup_open);
        assert!(state.filter_suggestion_selected > 0);
        let selected = state.filter_suggestions[state.filter_suggestion_selected]
            .text
            .clone();
        let expected = format!("name {selected} ");
        window.press("tab", cx);
        let state = view.read(cx);
        assert_eq!(state.filter_input.text(), expected);
        assert!(!state.filter_popup_open);
        assert!(state.filter_focus_handle.is_focused(window));
        assert!(state.filter_applied.is_none());
        // Once autocomplete closes, Tab must resume Kit's normal traversal.
        window.press("tab", cx);
        let state = view.read(cx);
        assert_eq!(state.filter_input.text(), expected);
        assert!(!state.filter_focus_handle.is_focused(window));
        assert!(state.filter_applied.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn filter_tab_without_candidates_and_shift_tab_keep_root_focus_traversal(
    cx: &mut TestAppContext,
) {
    for (text, key, has_candidates) in [("zzzz", "tab", false), ("name ", "shift-tab", true)] {
        let (_fixture, loaded) = csv_fixture();
        let (handle, view) = open_shell(cx, loaded);
        wait_for_grid(cx, handle, &view).await;
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("filter", cx);
        })
        .unwrap();
        cx.wait_for(handle.into(), WAIT, |window, cx| {
            view.read(cx).filter_focus_handle.is_focused(window)
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            window.input(text, cx);
            assert_eq!(!view.read(cx).filter_suggestions.is_empty(), has_candidates);
            window.press(key, cx);
            let state = view.read(cx);
            assert_eq!(state.filter_input.text(), text);
            assert!(!state.filter_focus_handle.is_focused(window));
            assert!(state.filter_applied.is_none());
        })
        .unwrap();
    }
}

fn archive_fixture() -> (tempfile::TempDir, Box<LoadedDocument>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("shell.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).expect("create ZIP"));
    for (name, bytes) in [
        ("first.csv", &b"name,value\nfirst,007\n"[..]),
        ("nested/second.csv", &b"name,value\nsecond,1.10\n"[..]),
        ("notes.md", &b"not a delimited dataset"[..]),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .expect("start ZIP entry");
        zip.write_all(bytes).expect("write ZIP entry");
    }
    zip.finish().expect("finish ZIP");
    let anvil_core::source::OpenResult::Archive(source) =
        anvil_core::source::open_path(&path).expect("discover ZIP")
    else {
        panic!("ZIP must be an archive");
    };
    let source = Arc::new(source);
    let loaded = anvil_core::loader::load_entry(&source, "first.csv").expect("load first entry");
    (dir, Box::new(loaded))
}

async fn wait_for_grid(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    view: &Entity<MainView>,
) {
    cx.wait_for(handle.into(), WAIT, |window, cx| {
        view.read(cx).focus_handle.is_focused(window)
    })
    .await;
}

fn assert_shared_active(view: &MainView) {
    let tab = &view.documents[view.active.expect("active tab")];
    assert!(Arc::ptr_eq(
        &view.model.as_ref().expect("active grid").doc,
        tab.doc.as_ref().expect("loaded tab")
    ));
}

#[gpui_kit::test]
async fn toolbar_clicks_open_real_query_bars_and_focus_in_both_themes(cx: &mut TestAppContext) {
    let (_fixture, loaded) = csv_fixture();
    let (handle, view) = open_shell(cx, loaded);
    wait_for_grid(cx, handle, &view).await;
    let original = cx.update(|cx| view.read(cx).model.as_ref().unwrap().doc.clone());

    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        if mode == ThemeMode::Light {
            cx.update_window(handle.into(), |_, window, cx| {
                window.click("appearance", cx)
            })
            .unwrap();
        }
        for (tool, close, text) in [
            ("find", "search-close", "alpha"),
            ("filter", "filter-close", "name = alpha"),
            ("ask", "ask-close", "show alpha"),
        ] {
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(cx.theme().mode, mode);
                window.click(tool, cx);
            })
            .unwrap();
            cx.wait_for(handle.into(), WAIT, |window, cx| {
                let view = view.read(cx);
                let focused = match tool {
                    "find" => view.search_open && view.search_focus_handle.is_focused(window),
                    "filter" => view.filter_open && view.filter_focus_handle.is_focused(window),
                    _ => view.ask_open && view.ask_focus_handle.is_focused(window),
                };
                focused && window.try_find(close).is_some()
            })
            .await;
            cx.update_window(handle.into(), |_, window, cx| {
                assert!(window.find(close).visible());
                // Filter and Ask deliberately retain text across dismissal.
                window.press(
                    if cfg!(target_os = "macos") {
                        "cmd-a"
                    } else {
                        "ctrl-a"
                    },
                    cx,
                );
                window.input(text, cx);
                let state = view.read(cx);
                assert_eq!(
                    match tool {
                        "find" => state.search_query.text(),
                        "filter" => state.filter_input.text(),
                        _ => state.ask_input.text(),
                    },
                    text
                );
                assert_eq!(state.documents.len(), 1);
                assert_shared_active(state);
                assert!(Arc::ptr_eq(&original, &state.model.as_ref().unwrap().doc));
                if tool == "filter" && state.filter_popup_open {
                    // Autocomplete owns the first Escape; the editor stays open.
                    window.press("escape", cx);
                    let state = view.read(cx);
                    assert!(!state.filter_popup_open);
                    assert!(state.filter_open);
                    assert!(state.filter_focus_handle.is_focused(window));
                    assert!(window.find(close).visible());
                }
                window.press("escape", cx);
            })
            .unwrap();
            wait_for_grid(cx, handle, &view).await;
            cx.update_window(handle.into(), |_, window, _| {
                assert!(window.try_find(close).is_none());
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
async fn appearance_button_round_trips_registered_palettes_without_copying_data(
    cx: &mut TestAppContext,
) {
    let (_fixture, loaded) = csv_fixture();
    let (handle, view) = open_shell(cx, loaded);
    wait_for_grid(cx, handle, &view).await;
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let original = view.read(cx).model.as_ref().unwrap().doc.clone();
        let dark = theme::palette(cx);
        assert_eq!(cx.theme().mode, ThemeMode::Dark);
        assert_eq!(cx.theme().dark_theme.name.as_ref(), "Anvil Ink");
        window.click("appearance", cx);
        assert_eq!(cx.theme().mode, ThemeMode::Light);
        assert_eq!(cx.theme().light_theme.name.as_ref(), "Anvil Vellum");
        let light = theme::palette(cx);
        assert_ne!(dark.canvas, light.canvas);
        assert_ne!(dark.foreground, light.foreground);
        window.click("appearance", cx);
        assert_eq!(cx.theme().mode, ThemeMode::Dark);
        assert_eq!(theme::palette(cx).canvas, dark.canvas);
        assert_shared_active(view.read(cx));
        assert!(Arc::ptr_eq(
            &original,
            &view.read(cx).model.as_ref().unwrap().doc
        ));
        assert_eq!(view.read(cx).documents.len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn document_menu_escape_restores_workspace_keyboard_focus(cx: &mut TestAppContext) {
    let (_fixture, loaded) = csv_fixture();
    let (handle, view) = open_shell(cx, loaded);
    wait_for_grid(cx, handle, &view).await;
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("document-menu", cx);
        assert_eq!(window.find("popup-menu").focused(), Some(true));
        assert!(!view.read(cx).focus_handle.is_focused(window));
        window.press("escape", cx);
    })
    .unwrap();
    wait_for_grid(cx, handle, &view).await;
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(window.try_find("popup-menu").is_none());
        // Prove the restored dispatch path is usable, not just a stored flag.
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-f"
            } else {
                "ctrl-f"
            },
            cx,
        );
    })
    .unwrap();
    cx.wait_for(handle.into(), WAIT, |window, cx| {
        view.read(cx).search_focus_handle.is_focused(window)
            && window.try_find("search-close").is_some()
    })
    .await;
}

async fn browse_archive(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("document-menu", cx);
        let mut menu = window.within("popup-menu");
        // Kit retains separator positions in the menu's item indices.
        assert_eq!(menu.find(4usize).label(), Some("Browse archive…"));
        menu.click(4usize, cx);
    })
    .unwrap();
    cx.wait_for(handle.into(), WAIT, |window, _| {
        window.try_find("dialog").is_some()
    })
    .await;
}

#[gpui_kit::test]
async fn archive_menu_dialog_dismisses_and_lazily_selects_existing_tab(cx: &mut TestAppContext) {
    let (_fixture, loaded) = archive_fixture();
    let (handle, view) = open_shell(cx, loaded);
    wait_for_grid(cx, handle, &view).await;
    let (ids, first) = cx.update(|cx| {
        let state = view.read(cx);
        assert_eq!(state.documents.len(), 2);
        assert_eq!(state.documents[0].entry_name.as_deref(), Some("first.csv"));
        assert_eq!(
            state.documents[1].entry_name.as_deref(),
            Some("nested/second.csv")
        );
        assert!(
            state.documents[1].doc.is_none(),
            "second entry must still be lazy"
        );
        assert_shared_active(state);
        (
            state.documents.iter().map(|tab| tab.id).collect::<Vec<_>>(),
            state.documents[0].doc.clone().unwrap(),
        )
    });

    browse_archive(cx, handle).await;
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(!view.read(cx).focus_handle.is_focused(window));
        assert!(view.read(cx).archive_dialog_open);
        window.press("escape", cx);
    })
    .unwrap();
    wait_for_grid(cx, handle, &view).await;
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(window.try_find("dialog").is_none());
        assert!(!view.read(cx).show_picker);
        assert!(!view.read(cx).archive_dialog_open);
        assert!(view.read(cx).documents[1].doc.is_none());
    })
    .unwrap();

    browse_archive(cx, handle).await;
    cx.update_window(handle.into(), |_, window, cx| {
        window
            .within("dialog")
            .click("archive-entry:nested/second.csv", cx);
    })
    .unwrap();
    cx.wait_for(handle.into(), WAIT, |window, cx| {
        let state = view.read(cx);
        state.active == Some(1)
            && state.documents[1].doc.is_some()
            && state.focus_handle.is_focused(window)
            && window.try_find("dialog").is_none()
    })
    .await;
    cx.update_window(handle.into(), |_, _, cx| {
        let state = view.read(cx);
        assert!(!state.show_picker);
        assert!(!state.archive_dialog_open);
        assert_eq!(
            state.documents.iter().map(|tab| tab.id).collect::<Vec<_>>(),
            ids
        );
        assert!(Arc::ptr_eq(
            &first,
            state.documents[0].doc.as_ref().unwrap()
        ));
        assert_shared_active(state);
        let second = state.documents[1].doc.as_ref().unwrap();
        assert!(!Arc::ptr_eq(&first, second));
        // Parsed record indices address data rows, excluding the header.
        assert_eq!(second.record_count(), 1);
        assert_eq!(
            second.cell(0, ColumnIndex(0)).unwrap().as_bytes(),
            b"second"
        );
        assert_eq!(second.cell(0, ColumnIndex(1)).unwrap().as_bytes(), b"1.10");
    })
    .unwrap();
}

#[gpui_kit::test]
async fn shortcuts_dialog_escape_restores_focus_and_keyboard_dispatch(cx: &mut TestAppContext) {
    let (_fixture, loaded) = csv_fixture();
    let (handle, view) = open_shell(cx, loaded);
    wait_for_grid(cx, handle, &view).await;
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("shortcuts", cx);
    })
    .unwrap();
    cx.wait_for(handle.into(), WAIT, |window, _| {
        window.try_find("dialog").is_some()
    })
    .await;
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(window.find("dialog").visible());
        assert!(!view.read(cx).focus_handle.is_focused(window));
        window.press("escape", cx);
    })
    .unwrap();
    wait_for_grid(cx, handle, &view).await;
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(window.try_find("dialog").is_none());
        assert_shared_active(view.read(cx));
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-l"
            } else {
                "ctrl-l"
            },
            cx,
        );
    })
    .unwrap();
    cx.wait_for(handle.into(), WAIT, |window, cx| {
        view.read(cx).filter_focus_handle.is_focused(window)
            && window.try_find("filter-close").is_some()
    })
    .await;
}

#[gpui_kit::test]
async fn shortcuts_keys_open_dialog_from_grid_and_query_and_restore_focus(cx: &mut TestAppContext) {
    let (_fixture, loaded) = csv_fixture();
    let (handle, view) = open_shell(cx, loaded);
    wait_for_grid(cx, handle, &view).await;
    let keys = if cfg!(target_os = "macos") {
        ["cmd-?", "cmd-shift-/"]
    } else {
        ["ctrl-?", "ctrl-shift-/"]
    };
    for from_query in [false, true] {
        if from_query {
            cx.update_window(handle.into(), |_, window, cx| window.click("find", cx))
                .unwrap();
            cx.wait_for(handle.into(), WAIT, |window, cx| {
                view.read(cx).search_focus_handle.is_focused(window)
                    && window.try_find("search-close").is_some()
            })
            .await;
        }
        for key in keys {
            cx.update_window(handle.into(), |_, window, cx| window.press(key, cx))
                .unwrap();
            cx.wait_for(handle.into(), WAIT, |window, _| {
                window.try_find("dialog").is_some()
            })
            .await;
            cx.update_window(handle.into(), |_, window, cx| {
                assert!(window.find("dialog").visible());
                let state = view.read(cx);
                assert!(!state.focus_handle.is_focused(window));
                assert!(!state.search_focus_handle.is_focused(window));
                window.press("escape", cx);
            })
            .unwrap();
            cx.wait_for(handle.into(), WAIT, |window, cx| {
                let state = view.read(cx);
                let restored = if from_query {
                    state.search_open && state.search_focus_handle.is_focused(window)
                } else {
                    state.focus_handle.is_focused(window)
                };
                restored && window.try_find("dialog").is_none()
            })
            .await;
            cx.update_window(handle.into(), |_, window, cx| {
                let state = view.read(cx);
                assert_eq!(state.documents.len(), 1);
                assert_shared_active(state);
                if from_query {
                    // The restored editor remains usable, and ? did not insert text.
                    assert!(state.search_query.is_empty());
                    window.input("alpha", cx);
                    assert_eq!(view.read(cx).search_query.text(), "alpha");
                    window.press(
                        if cfg!(target_os = "macos") {
                            "cmd-a"
                        } else {
                            "ctrl-a"
                        },
                        cx,
                    );
                    window.press("backspace", cx);
                }
            })
            .unwrap();
        }
    }
}

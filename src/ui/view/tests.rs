//! Unit and headless UI tests for `MainView`.
//!
//! Kept in a sibling file so `view.rs` stays focused on the
//! production view; these are compiled only under `cfg(test)`.

use super::search::search_column_label;
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

/// A document with `cols` columns and three rows. The string `needle`
/// is placed only in `needle_col` of row 1, so a whole-record search
/// must discover that column.
fn wide_document(cols: usize, needle_col: usize) -> anvil_core::ParsedDocument {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("wide.csv");
    let mut csv = String::new();
    csv.push_str(
        &(0..cols)
            .map(|c| format!("c{c}"))
            .collect::<Vec<_>>()
            .join(","),
    );
    csv.push('\n');
    for row in 0..3 {
        let cells: Vec<String> = (0..cols)
            .map(|c| {
                if row == 1 && c == needle_col {
                    "needle".to_string()
                } else {
                    format!("v{row}{c}")
                }
            })
            .collect();
        csv.push_str(&cells.join(","));
        csv.push('\n');
    }
    std::fs::write(&path, csv).expect("write fixture");
    let opened = anvil_core::source::local::open_local(&path).expect("open local");
    anvil_core::index_opened(opened)
}

/// A document with two row-level warnings: an inconsistent-width row
/// (record 1) followed by an unterminated quote (record 2).
fn warning_document() -> anvil_core::ParsedDocument {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("warn.csv");
    std::fs::write(&path, b"a,b,c\n1,2,3\n4,5\n6,\"unterminated\n").expect("write fixture");
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

/// Cmd/Ctrl+Home / Cmd/Ctrl+End jump to the first / last row while
/// keeping the current column. Fails if the chord is bound to the
/// plain column jump instead.
#[gpui::test]
async fn cmd_home_and_end_jump_to_row_extremes(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, large_document(50));

    // Put the cursor at (1, 1).
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("right");
    let home = if cfg!(target_os = "macos") {
        "cmd-home"
    } else {
        "ctrl-home"
    };
    let end = if cfg!(target_os = "macos") {
        "cmd-end"
    } else {
        "ctrl-end"
    };

    cx.simulate_keystrokes(end);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.selection.cursor(), Some(CellPos::new(49, 1)));
    });

    cx.simulate_keystrokes(home);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.selection.cursor(), Some(CellPos::new(0, 1)));
    });
}

/// Tab / Shift-Tab move one cell in row-major order and wrap across
/// row boundaries. Fails if the grid stops receiving Tab events
/// (e.g. focus traversal swallows them) or the wrap is off by one.
#[gpui::test]
async fn tab_and_shift_tab_move_adjacent_cells(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);

    // sample.csv has 3 columns and 3 data rows.
    cx.update_window_entity(&view, |view, _, cx| {
        view.selection.set(CellPos::new(0, 2));
        cx.notify();
    });
    cx.simulate_keystrokes("tab");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.selection.cursor(), Some(CellPos::new(1, 0)));
    });

    cx.simulate_keystrokes("shift-tab");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.selection.cursor(), Some(CellPos::new(0, 2)));
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
        assert_eq!(view.search_query.text(), "");
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
        assert_eq!(view.search_query.text(), "foo");
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
        assert_eq!(view.search_query.text(), "fo");
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
        assert_eq!(view.search_query.text(), "");
        assert_eq!(view.search_matches, Vec::<u64>::new());
        assert_eq!(view.search_active, -1);
        // We don't assert on `focus_grid_on_next_frame` here: that
        // flag is cleared as soon as `render_grid` runs (the
        // deferred focus consumes it). The contract we care about
        // is "the bar is closed"; the next user keystroke will
        // route through the grid handler.
    });
}

/// Esc closes the filter bar while it holds focus.
#[gpui::test]
async fn escape_closes_filter_bar(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.filter_open, "filter bar should be closed");
        assert!(view.filter_error.is_none());
    });
}

/// Esc must also close the filter bar through the grid's `Action::Escape`
/// path. That arm previously handled only the search bar, so once focus
/// was back on the grid the filter was impossible to dismiss.
#[gpui::test]
async fn escape_action_closes_filter_bar_when_grid_focused(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.run_until_parked();
    // Move focus back to the grid, as a click on a cell would.
    cx.update_window_entity(&view, |view, window, _| window.focus(&view.focus_handle));
    cx.run_until_parked();

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(
            !view.filter_open,
            "filter bar should close from the grid's Escape action"
        );
    });
}

/// The visible × dismisses the filter bar (in addition to Esc).
#[gpui::test]
async fn filter_close_button_dismisses_bar(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.run_until_parked();

    let bounds = cx
        .debug_bounds("filter-close")
        .expect("filter close button");
    let center = bounds.center();
    let modifiers = gpui::Modifiers::none();
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, modifiers);
    cx.run_until_parked();

    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.filter_open, "clicking × should close the filter bar");
    });
}

/// The visible × dismisses the search bar.
#[gpui::test]
async fn search_close_button_dismisses_bar(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
    cx.run_until_parked();

    let bounds = cx
        .debug_bounds("search-close")
        .expect("search close button");
    let center = bounds.center();
    let modifiers = gpui::Modifiers::none();
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, modifiers);
    cx.run_until_parked();

    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.search_open, "clicking × should close the search bar");
    });
}

/// Typing in the filter bar opens the autocomplete popup with the
/// matching column.
#[gpui::test]
async fn filter_suggestions_open_on_typing(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("c");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.filter_popup_open, "popup should open on typing");
        assert_eq!(
            view.filter_suggestions.first().map(|s| s.text.as_str()),
            Some("country")
        );
    });
}

/// Up/Down move the popup selection and Enter fills the highlighted
/// suggestion (plus a separator) and closes the popup.
#[gpui::test]
async fn filter_suggestions_arrow_and_enter_accept(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_filter(cx);
        view.refresh_filter_suggestions();
        view.filter_popup_open = true;
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_suggestions.len(), 4);
        assert_eq!(view.filter_suggestion_selected, 0);
    });

    cx.simulate_keystrokes("down");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_suggestion_selected, 1);
    });
    cx.simulate_keystrokes("up");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_suggestion_selected, 0);
    });
    // Skip the `any` / `all` row-scope pseudo-columns to reach `country`.
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("down");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_suggestion_selected, 2);
    });

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_input.text(), "country ");
        assert!(!view.filter_popup_open);
    });
}

/// Accepting a column whose name contains a space inserts it quoted, so
/// it stays one token and the value suggestions for that column populate.
#[gpui::test]
async fn accepting_a_multi_word_column_suggestion_quotes_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cities.csv");
    std::fs::write(&path, b"New York,pad\nNYC,x\nLA,x\n").expect("write");
    let opened = anvil_core::source::local::open_local(&path).expect("open");
    let doc = anvil_core::index_opened(opened);

    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, doc);
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_filter(cx);
        view.filter_input = "New".into();
        view.refresh_filter_suggestions();
        view.filter_popup_open = true;
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(
            view.filter_suggestions.first().map(|s| s.text.as_str()),
            Some("New York")
        );
    });

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_input.text(), "\"New York\" ");
        // Complete the predicate; the quoted column must resolve, so its
        // distinct values are offered.
        view.filter_input = "\"New York\" == ".into();
        view.refresh_filter_suggestions();
        let texts: Vec<&str> = view
            .filter_suggestions
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert!(texts.contains(&"NYC"), "value suggestions: {texts:?}");
        assert!(texts.contains(&"LA"), "value suggestions: {texts:?}");
    });
}

/// Moving the selection past the popup's visible area must scroll the
/// popup so the highlighted row stays in view (regression: it did not).
#[gpui::test]
async fn filter_suggestions_scroll_to_keep_selection_visible(cx: &mut TestAppContext) {
    // A column with many distinct values so the value slot overflows the
    // popup's 200px max height.
    let mut csv = String::from("k,pad\n");
    for i in 0..50 {
        csv.push_str(&format!("{i},x\n"));
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("many.csv");
    std::fs::write(&path, csv.as_bytes()).expect("write");
    let opened = anvil_core::source::local::open_local(&path).expect("open");
    let doc = anvil_core::index_opened(opened);

    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, doc);
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_filter(cx);
        view.filter_input = "k == ".into();
        view.refresh_filter_suggestions();
        view.filter_popup_open = true;
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(
            view.filter_suggestions.len() > 10,
            "need enough suggestions to overflow the popup"
        );
        assert_eq!(f32::from(view.filter_suggestion_scroll.offset().y), 0.0);
    });

    for _ in 0..15 {
        cx.simulate_keystrokes("down");
    }
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_suggestion_selected, 15);
        // GPUI scroll offsets are negative when scrolled down.
        let off = f32::from(view.filter_suggestion_scroll.offset().y);
        assert!(
            off < 0.0,
            "popup should scroll to keep the selection visible: offset={off}"
        );
    });
}

/// Esc closes the autocomplete popup first; a second Esc closes the bar.
#[gpui::test]
async fn filter_suggestions_escape_closes_popup_then_bar(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_filter(cx);
        view.filter_input = "c".into();
        view.refresh_filter_suggestions();
        view.filter_popup_open = true;
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.filter_popup_open);
        assert!(view.filter_open, "first Esc only closes the popup");
    });

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.filter_open);
    });
}

/// Clicking a popup row fills the input like accepting it with Enter.
#[gpui::test]
async fn filter_suggestion_mouse_click_fills_the_input(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_filter(cx);
        view.refresh_filter_suggestions();
        view.filter_popup_open = true;
    });
    cx.run_until_parked();

    let bounds = cx
        .debug_bounds("filter-suggestion-3")
        .expect("suggestion row 3");
    let center = bounds.center();
    let modifiers = gpui::Modifiers::none();
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, modifiers);
    cx.run_until_parked();

    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_input.text(), "amount ");
        assert!(!view.filter_popup_open);
    });
}

/// Switching the active document drops the cached value samples.
#[gpui::test]
async fn switching_document_clears_the_value_cache(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, _| {
        view.filter_value_cache.insert(0, vec!["US".to_string()]);
        view.load_active();
        assert!(view.filter_value_cache.is_empty());
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

/// The `Aa` toggle switches the active search between
/// case-insensitive and byte-exact, and re-runs the open search so
/// the match list follows the option.
#[gpui::test]
async fn search_case_sensitive_toggle_changes_matches(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("case.csv");
    std::fs::write(&path, b"name,tag\nAlice,x\nalice,y\nALICE,z\n").expect("write");
    let opened = anvil_core::source::local::open_local(&path).expect("open");
    let doc = anvil_core::index_opened(opened);
    inject_doc(cx, &view, doc);

    cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("alice");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.search_case_sensitive);
        assert_eq!(view.search_matches, vec![0, 1, 2]);
    });

    cx.update_window_entity(&view, |view, _, cx| view.toggle_search_case_sensitive(cx));
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.search_case_sensitive);
        // Only the lowercase `alice` row matches byte-exactly.
        assert_eq!(view.search_matches, vec![1]);
    });
}

/// A column scope restricts the search to one column, and the
/// active-match jump lands the cursor in that column.
#[gpui::test]
async fn search_column_scope_limits_matches(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("scope.csv");
    std::fs::write(&path, b"a,b\nUS,x\nUK,US\n").expect("write");
    let opened = anvil_core::source::local::open_local(&path).expect("open");
    let doc = anvil_core::index_opened(opened);
    inject_doc(cx, &view, doc);

    cx.update_window_entity(&view, |view, _, cx| {
        view.search_column = Some(1);
        view.open_search(cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("US");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        // `US` only appears in column 1 of row 1 when scoped.
        assert_eq!(view.search_matches, vec![1]);
        assert_eq!(view.selection.cursor(), Some(CellPos::new(1, 1)));
    });
}

/// Clicking a column header toggles the search column scope; a
/// second click on the same header clears it. Exercises the real
/// `grid::render_header` click wiring via the test debug selector.
#[gpui::test]
async fn alt_header_click_toggles_search_column(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);

    let bounds = cx.debug_bounds("header-cell-1").expect("header cell 1");
    let center = gpui::point(
        bounds.origin.x + bounds.size.width / 2.0,
        bounds.origin.y + bounds.size.height / 2.0,
    );
    // Alt/Option-click sets the search scope; a plain click sorts.
    let modifiers = gpui::Modifiers {
        alt: true,
        ..Default::default()
    };

    cx.simulate_mouse_down(center, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, modifiers);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.search_column, Some(1));
    });

    cx.simulate_mouse_down(center, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, modifiers);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.search_column, None);
    });
}

/// A plain header click cycles the sort none -> ascending -> descending
/// -> none. The sample document's column `a` is 1, 4, 7 (numeric), so
/// the sorted permutations are easy to assert.
#[gpui::test]
async fn header_click_cycles_sort(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);

    let bounds = cx.debug_bounds("header-cell-0").expect("header cell 0");
    let center = gpui::point(
        bounds.origin.x + bounds.size.width / 2.0,
        bounds.origin.y + bounds.size.height / 2.0,
    );
    let modifiers = gpui::Modifiers::default();

    let click = |cx: &mut VisualTestContext| {
        cx.simulate_mouse_down(center, gpui::MouseButton::Left, modifiers);
        cx.simulate_mouse_up(center, gpui::MouseButton::Left, modifiers);
        cx.run_until_parked();
    };

    // First click: ascending.
    click(cx);
    cx.update_window_entity(&view, |view, _, _| {
        let spec = view.sort_spec.expect("sort should be active");
        assert_eq!(spec.column, 0);
        assert_eq!(spec.order, anvil_core::SortOrder::Ascending);
        assert_eq!(
            view.sort_rows.as_ref().map(|rows| rows.as_slice()),
            Some(&[0u64, 1, 2][..])
        );
    });

    // Second click: descending.
    click(cx);
    cx.update_window_entity(&view, |view, _, _| {
        let spec = view.sort_spec.expect("sort should be active");
        assert_eq!(spec.order, anvil_core::SortOrder::Descending);
        assert_eq!(
            view.sort_rows.as_ref().map(|rows| rows.as_slice()),
            Some(&[2u64, 1, 0][..])
        );
    });

    // Third click: cleared.
    click(cx);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.sort_spec.is_none());
        assert!(view.sort_rows.is_none());
    });
}

/// Sorting composes with an active filter: the sort permutation walks
/// the filtered indexes, not the whole document.
///
/// `country_document` rows are (US,10), (UK,20), (US,30). Filtering to
/// the US rows leaves records 0 and 2; sorting those by `amount`
/// descending gives 2 then 0.
#[gpui::test]
async fn sort_composes_with_filter(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.update_window_entity(&view, |view, _, cx| {
        view.filter_input = "country == US".into();
        view.apply_filter_input(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, cx| view.close_filter(cx));

    // Ascending on `amount`: 10 then 30 -> records 0 then 2.
    cx.update_window_entity(&view, |view, _, cx| view.cycle_sort(1, cx));
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(
            view.sort_rows.as_ref().map(|rows| rows.as_slice()),
            Some(&[0u64, 2][..])
        );
    });

    // Descending: 30 then 10 -> records 2 then 0.
    cx.update_window_entity(&view, |view, _, cx| view.cycle_sort(1, cx));
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(
            view.sort_rows.as_ref().map(|rows| rows.as_slice()),
            Some(&[2u64, 0][..])
        );
    });
}

/// `search_options()` is the single source of truth shared by the
/// background search and the grid highlight pass.
#[gpui::test]
async fn search_options_reflect_toggles(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.search_options(), anvil_core::SearchOptions::default());
        view.search_case_sensitive = true;
        view.search_column = Some(2);
        assert_eq!(
            view.search_options(),
            anvil_core::SearchOptions {
                case_sensitive: true,
                column: Some(2),
                max_matches: None,
            }
        );
    });
}

#[test]
fn search_column_label_resolves_headers_and_fallback() {
    let headers = vec!["id".to_string(), String::new(), "name".to_string()];
    assert_eq!(search_column_label(&headers, 0), "id");
    assert_eq!(search_column_label(&headers, 1), "col1");
    assert_eq!(search_column_label(&headers, 2), "name");
    assert_eq!(search_column_label(&headers, 9), "col9");
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
        assert_eq!(
            view.search_query.text(),
            "hit",
            "F3 should not edit the query"
        );
        assert_eq!(view.search_active, 1);
    });

    // Shift+F3 -> previous.
    cx.simulate_keystrokes("shift-f3");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.search_active, 0);
    });
}

/// The search field supports caret movement and Shift selection,
/// matching the filter field's behaviour.
#[gpui::test]
async fn search_keyboard_caret_and_selection(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_search(cx);
        view.search_query = "needle".into();
    });
    cx.run_until_parked();
    let len = "needle".len();

    cx.simulate_keystrokes("home");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.search_query.caret(), 0);
    });
    cx.simulate_keystrokes("end");
    cx.simulate_keystrokes("left");
    cx.simulate_keystrokes("shift-home");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.search_query.selection(), 0..len - 1);
        assert_eq!(view.search_query.selected_text(), "needl");
    });
}

/// Clicking and dragging in the search field place and extend the
/// caret, exactly like the filter field.
#[gpui::test]
async fn search_mouse_click_and_drag(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_search(cx);
        view.search_query = "needle".into();
    });
    cx.run_until_parked();
    let len = "needle".len();

    let bounds = cx
        .debug_bounds("anvil-search-input")
        .expect("search input bounds");
    let y = bounds.origin.y + bounds.size.height / 2.0;
    let left = gpui::point(bounds.origin.x + px(1.0), y);
    let right = gpui::point(bounds.origin.x + bounds.size.width - px(1.0), y);
    let mid = gpui::point(bounds.origin.x + bounds.size.width / 2.0, y);
    let modifiers = gpui::Modifiers::default();

    cx.simulate_mouse_down(right, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(right, gpui::MouseButton::Left, modifiers);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.search_query.caret(), len);
    });

    cx.simulate_mouse_down(left, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_move(mid, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(mid, gpui::MouseButton::Left, modifiers);
    cx.update_window_entity(&view, |view, _, _| {
        let selection = view.search_query.selection();
        assert_eq!(selection.start, 0);
        assert!(
            selection.end > 0 && selection.end <= len,
            "drag should select a non-empty prefix, got {selection:?}"
        );
    });
}

/// Typing at a mid-string caret inserts there and re-runs the live
/// search with the new query.
#[gpui::test]
async fn search_insert_at_caret_re_runs_search(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("insert.csv");
    std::fs::write(&path, b"a,b\nneedle,1\nx,needle\nother,foo\n").expect("write");
    let opened = anvil_core::source::local::open_local(&path).expect("open");
    let doc = anvil_core::index_opened(opened);
    inject_doc(cx, &view, doc);

    cx.update_window_entity(&view, |view, _, cx| {
        view.open_search(cx);
        view.search_query = "ndle".into();
        view.search_query.set_caret(1, false);
        view.kick_search(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.search_matches, Vec::<u64>::new());
        assert_eq!(view.search_query.caret(), 1);
    });

    cx.simulate_keystrokes("e e");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.search_query.text(), "needle");
        assert_eq!(view.search_query.caret(), 3);
        assert_eq!(view.search_matches, vec![0, 1]);
    });
}

/// Jumping to a search match that lives in a far-right column scrolls
/// the grid horizontally so the matched cell is on screen. Uses the
/// real (panel-aware) viewport rather than forcing one, so it also
/// guards the file-panel inset.
#[gpui::test]
async fn search_jump_scrolls_horizontally_to_match(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    // 300 columns is far wider than any test window.
    inject_doc(cx, &view, wide_document(300, 299));

    cx.update_window_entity(&view, |view, _, cx| {
        view.open_search(cx);
        view.search_query = "needle".into();
        view.kick_search(cx);
    });
    cx.run_until_parked();

    cx.update_window_entity(&view, |view, _, cx| {
        view.scroll_x = 0.0;
        view.jump_to_active_match(cx);

        let cursor = view.selection.cursor().expect("match cursor");
        assert_eq!(
            cursor.column_index, 299,
            "should land on the matching column"
        );
        let layout = &view.model.as_ref().expect("model").layout;
        let start = layout.offsets[cursor.column_index];
        let width = layout.widths[cursor.column_index];
        assert!(view.scroll_x > 0.0, "should have scrolled right");
        assert!(
            start >= view.scroll_x && start + width <= view.scroll_x + view.viewport_w,
            "cell not fully visible: start={start} width={width} scroll={} viewport={}",
            view.scroll_x,
            view.viewport_w
        );
    });
}

/// Regression: the grid viewport must include exactly the panels that
/// share its flex row, and it must update in the *same* render a panel is
/// shown or hidden. Before the viewport was computed synchronously it came
/// from the previous frame's measured `grid_bounds`, so closing a panel
/// left the table at the old width until a later frame re-measured - a
/// visible gap. A single `notify` + render now settles it.
#[gpui::test]
async fn viewport_width_reflows_synchronously_with_panels(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);

    let window_w = cx.update_window_entity(&view, |_, window, _| {
        f32::from(window.viewport_size().width)
    });
    let gutter = f32::from(theme::ROW_NUMBER_WIDTH);

    let assert_width = |view: &MainView, expected: f32| {
        assert!(
            (view.viewport_w - expected).abs() < 0.5,
            "viewport_w={} expected {expected}",
            view.viewport_w
        );
    };

    // File panel open (the default once a document exists).
    cx.update_window_entity(&view, |view, _, _| {
        assert_width(view, window_w - gutter - f32::from(theme::FILE_PANEL_WIDTH));
    });

    // Opening the inspector narrows the grid in the same frame.
    cx.update_window_entity(&view, |view, _, cx| {
        view.inspector_open = true;
        cx.notify();
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_width(
            view,
            window_w
                - gutter
                - f32::from(theme::FILE_PANEL_WIDTH)
                - f32::from(theme::INSPECTOR_WIDTH),
        );
    });

    // Diagnostics adds its own width.
    cx.update_window_entity(&view, |view, _, cx| {
        view.diagnostics_open = true;
        cx.notify();
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_width(
            view,
            window_w
                - gutter
                - f32::from(theme::FILE_PANEL_WIDTH)
                - f32::from(theme::INSPECTOR_WIDTH)
                - f32::from(theme::DIAGNOSTICS_WIDTH),
        );
    });

    // Closing everything widens the grid back immediately.
    cx.update_window_entity(&view, |view, _, cx| {
        view.file_panel_open = false;
        view.inspector_open = false;
        view.diagnostics_open = false;
        cx.notify();
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_width(view, window_w - gutter);
    });
}

/// Scrolling by display position must map through the filtered/sorted
/// view, not raw record indexes.
#[gpui::test]
async fn display_position_maps_through_view_rows(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);

    cx.update_window_entity(&view, |view, _, _| {
        // No filter/sort: identity.
        assert_eq!(view.display_position(5), 5);
        // A filtered view reorders display positions.
        view.filter_rows = Some(std::sync::Arc::new(vec![2, 4, 6]));
        assert_eq!(view.display_position(2), 0);
        assert_eq!(view.display_position(4), 1);
        assert_eq!(view.display_position(6), 2);
        // A record not present falls back to the first row.
        assert_eq!(view.display_position(0), 0);
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
        assert_eq!(view.search_query.text(), "");
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
        view.search_case_sensitive = true;
        view.search_column = Some(2);
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
        assert!(!view.search_case_sensitive);
        assert_eq!(view.search_column, None);
    });

    // Switch back to tab 0 - the selection comes back.
    cx.update_window_entity(&view, |view, _, cx| {
        view.switch_to(0, cx);
    });
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.active, Some(0));
        assert_eq!(view.selection.cursor(), Some(CellPos::new(1, 1)));
        assert_eq!(view.scroll_x, 42.0);
        assert!(view.search_case_sensitive);
        assert_eq!(view.search_column, Some(2));
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
/// drives the real `apply(OpenOutcome::Ready)` path twice - the
/// same code that runs after `begin_open`'s background task - so
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

/// Cmd/Ctrl+B toggles the file-manager panel. The binding routes
/// through `on_grid_key_down` like every other grid shortcut, so
/// this also re-verifies the keyboard path.
#[gpui::test]
async fn cmd_b_toggles_file_panel(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.file_panel_open, "panel open by default");
    });

    let chord = if cfg!(target_os = "macos") {
        "cmd-b"
    } else {
        "ctrl-b"
    };
    cx.simulate_keystrokes(chord);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.file_panel_open, "first toggle hides it");
    });
    cx.simulate_keystrokes(chord);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.file_panel_open, "second toggle shows it again");
    });
}

/// Cmd/Ctrl+I toggles the column-inspector panel and scans the selected
/// column when it opens.
#[gpui::test]
async fn cmd_i_toggles_inspector(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.inspector_open, "inspector closed by default");
    });

    let chord = if cfg!(target_os = "macos") {
        "cmd-i"
    } else {
        "ctrl-i"
    };
    cx.simulate_keystrokes(chord);
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.inspector_open);
        assert_eq!(view.inspector_column, Some(0));
        assert!(view.inspector_stats.is_some());
    });

    cx.simulate_keystrokes(chord);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.inspector_open);
    });
}

/// The inspector follows the selected column: moving right rescans the
/// new column.
#[gpui::test]
async fn inspector_follows_selected_column(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.inspector_open = true;
        view.inspector_column = None;
        view.kick_inspector(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.inspector_column, Some(0));
        assert_eq!(
            view.inspector_stats.as_ref().expect("stats").name,
            "country"
        );
    });

    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        let stats = view.inspector_stats.as_ref().expect("stats");
        assert_eq!(view.inspector_column, Some(1));
        assert_eq!(stats.name, "amount");
        assert_eq!(stats.non_empty, 3);
    });
}

/// The inspector body data distinguishes exact distinct counts and
/// ranks frequent values.
#[gpui::test]
async fn inspector_reports_exact_distinct_and_frequencies(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.inspector_open = true;
        view.inspector_column = None;
        view.kick_inspector(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        let stats = view.inspector_stats.as_ref().expect("stats");
        assert_eq!(stats.distinct, anvil_core::DistinctCount::Exact(2));
        assert_eq!(stats.frequent.len(), 2);
        assert_eq!(stats.frequent[0].value, "US");
        assert_eq!(stats.frequent[0].count, 2);
    });
}

/// Cmd/Ctrl+D toggles the diagnostics panel.
#[gpui::test]
async fn cmd_d_toggles_diagnostics(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.diagnostics_open, "diagnostics closed by default");
    });

    let chord = if cfg!(target_os = "macos") {
        "cmd-d"
    } else {
        "ctrl-d"
    };
    cx.simulate_keystrokes(chord);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.diagnostics_open);
    });
    cx.simulate_keystrokes(chord);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.diagnostics_open);
    });
}

/// Clicking a row warning moves the cursor to that record; the panel is
/// a pure render over the document's existing `ParseMeta.warnings`.
#[gpui::test]
async fn diagnostics_navigation_moves_selection_to_warning_row(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, warning_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.diagnostics_open = true;
        assert_eq!(view.model.as_ref().unwrap().doc.meta.warnings.len(), 2);
        view.selection.set(CellPos::new(0, 0));

        view.jump_to_warning(0, cx);
        assert_eq!(view.selection.cursor().map(|c| c.record_index), Some(1));
        assert!(view.status_message.is_none());

        view.jump_to_warning(1, cx);
        assert_eq!(view.selection.cursor().map(|c| c.record_index), Some(2));
    });
}

/// A warning whose record is hidden by the active filter must not move
/// the cursor to an unrelated row; the status bar explains why.
#[gpui::test]
async fn diagnostics_navigation_refuses_a_filtered_out_row(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, warning_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.selection.set(CellPos::new(0, 0));
        view.filter_rows = Some(Arc::new(vec![0]));
        view.jump_to_warning(0, cx);
        assert_eq!(view.selection.cursor().map(|c| c.record_index), Some(0));
        let msg = view.status_message.clone().expect("status message");
        assert!(msg.contains("hidden"), "unexpected status: {msg}");
    });
}

/// A header-level warning has no row to jump to and reports that.
#[gpui::test]
async fn diagnostics_navigation_reports_header_warning(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let doc = {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty_header.csv");
        std::fs::write(&path, b"a,,c\n1,2,3\n").expect("write fixture");
        let opened = anvil_core::source::local::open_local(&path).expect("open local");
        anvil_core::index_opened(opened)
    };
    inject_doc(cx, &view, doc);
    cx.update_window_entity(&view, |view, _, cx| {
        assert_eq!(view.model.as_ref().unwrap().doc.meta.warnings.len(), 1);
        view.jump_to_warning(0, cx);
        let msg = view.status_message.clone().expect("status message");
        assert!(msg.contains("header"), "unexpected status: {msg}");
    });
}

/// Opening the diagnostics panel kicks a background UTF-8 scan; the
/// report then names the offending column.
#[gpui::test]
async fn diagnostics_scan_reports_invalid_utf8(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let doc = {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("encoding.csv");
        std::fs::write(&path, b"a,b\n1,\xff\n2,\xfe\n").expect("write fixture");
        let opened = anvil_core::source::local::open_local(&path).expect("open local");
        anvil_core::index_opened(opened)
    };
    inject_doc(cx, &view, doc);
    cx.update_window_entity(&view, |view, _, cx| {
        assert!(view.encoding_report.is_none());
        view.toggle_diagnostics(cx);
        assert!(view.diagnostics_open);
        assert!(view.encoding_task.is_some(), "scan kicked on open");
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        let report = view.encoding_report.as_ref().expect("encoding report");
        assert!(report.has_invalid_utf8());
        assert_eq!(report.total_invalid_cells(), 2);
        assert_eq!(report.columns[0].column, 1);
        assert_eq!(report.columns[0].invalid_cells, 2);
    });
}

/// Changing the active document drops the cached encoding report.
#[gpui::test]
async fn switching_document_invalidates_encoding_report(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let doc = {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("encoding.csv");
        std::fs::write(&path, b"a,b\n1,\xff\n").expect("write fixture");
        let opened = anvil_core::source::local::open_local(&path).expect("open local");
        anvil_core::index_opened(opened)
    };
    inject_doc(cx, &view, doc);
    cx.update_window_entity(&view, |view, _, cx| view.toggle_diagnostics(cx));
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.encoding_report.is_some());
    });

    // A second document resets the report; the panel stays open but
    // shows the new document's status.
    inject_doc(cx, &view, sample_document());
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.encoding_report.is_none());
    });
}

/// An ambiguous all-text file is a header under Auto; forcing headerless
/// re-indexes it so every row is data and columns get `colN` labels.
#[gpui::test]
async fn cycle_header_mode_reindexes_active_document(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let doc = {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ambiguous.csv");
        std::fs::write(&path, b"alice,bob\ncarol,dave\n").expect("write fixture");
        let opened = anvil_core::source::local::open_local(&path).expect("open local");
        anvil_core::index_opened(opened)
    };
    inject_doc(cx, &view, doc);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.header_mode, anvil_core::HeaderMode::Auto);
        assert!(view.model.as_ref().unwrap().doc.meta.has_headers);
        assert_eq!(view.model.as_ref().unwrap().row_count(), 1);
    });

    // Auto -> Header -> Headerless.
    cx.update_window_entity(&view, |view, _, cx| {
        view.cycle_header_mode(cx);
        view.cycle_header_mode(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.header_mode, anvil_core::HeaderMode::Headerless);
        let doc = &view.model.as_ref().unwrap().doc;
        assert!(!doc.meta.has_headers);
        assert_eq!(doc.record_count(), 2);
        assert_eq!(doc.meta.column_label(0), "col0");
        assert_eq!(doc.meta.column_label(1), "col1");
    });
}

/// Forcing a header when the row count shrinks drops an out-of-range
/// selection rather than leaving the cursor on a non-existent row.
#[gpui::test]
async fn header_toggle_clamps_selection(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let doc = {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ambiguous.csv");
        std::fs::write(&path, b"alice,bob\ncarol,dave\n").expect("write fixture");
        let opened = anvil_core::source::local::open_local(&path).expect("open local");
        anvil_core::index_opened(opened)
    };
    inject_doc(cx, &view, doc);

    cx.update_window_entity(&view, |view, _, cx| {
        view.apply_header_mode(anvil_core::HeaderMode::Headerless, cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.model.as_ref().unwrap().row_count(), 2);
        view.selection.set(CellPos::new(1, 0));
    });

    cx.update_window_entity(&view, |view, _, cx| {
        view.apply_header_mode(anvil_core::HeaderMode::Header, cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.model.as_ref().unwrap().row_count(), 1);
        assert!(view.selection.cursor().is_none(), "selection clamped away");
    });
}

/// The forced mode is per tab and survives switching away and back.
#[gpui::test]
async fn header_mode_persists_per_tab(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let doc = {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ambiguous.csv");
        std::fs::write(&path, b"alice,bob\ncarol,dave\n").expect("write fixture");
        let opened = anvil_core::source::local::open_local(&path).expect("open local");
        anvil_core::index_opened(opened)
    };
    inject_doc(cx, &view, doc);
    cx.update_window_entity(&view, |view, _, cx| {
        view.apply_header_mode(anvil_core::HeaderMode::Headerless, cx);
    });
    cx.run_until_parked();

    // A second tab; then switch back to the first.
    inject_doc(cx, &view, sample_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.switch_to(0, cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.header_mode, anvil_core::HeaderMode::Headerless);
        assert!(!view.model.as_ref().unwrap().doc.meta.has_headers);
        assert_eq!(view.model.as_ref().unwrap().row_count(), 2);
    });
}

/// `group_documents` groups direct files on their own and nests
/// ZIP entries under their archive, keyed by `Arc` identity so two
/// archives with the same file name stay separate.
#[test]
fn group_documents_groups_by_archive_identity() {
    let (_dir_a, archive_a) = make_zip_archive(&["a.csv", "b.csv"]);
    let (_dir_b, archive_b) = make_zip_archive(&["c.csv"]);
    let docs = vec![
        DocumentSnapshot::from_doc(0, Arc::new(sample_document()), None, None),
        DocumentSnapshot::pending_entry(1, archive_a.clone(), "a.csv".into()),
        DocumentSnapshot::pending_entry(2, archive_a.clone(), "b.csv".into()),
        DocumentSnapshot::pending_entry(3, archive_b.clone(), "c.csv".into()),
        DocumentSnapshot::from_doc(4, Arc::new(sample_document()), None, None),
    ];
    let groups = group_documents(&docs);

    assert_eq!(groups.len(), 4, "two direct files + two archives");
    assert!(groups[0].0.is_none(), "first direct file is top-level");
    assert_eq!(groups[0].1.len(), 1);
    assert_eq!(groups[0].1[0].0, 0);

    assert!(groups[1]
        .0
        .as_ref()
        .is_some_and(|a| Arc::ptr_eq(a, &archive_a)));
    assert_eq!(groups[1].1.len(), 2, "both archive_a entries grouped");
    assert_eq!(groups[1].1[0].0, 1);
    assert_eq!(groups[1].1[1].0, 2);

    assert!(groups[2]
        .0
        .as_ref()
        .is_some_and(|a| Arc::ptr_eq(a, &archive_b)));
    assert_eq!(groups[2].1.len(), 1);

    assert!(groups[3].0.is_none(), "last direct file is top-level");
    assert_eq!(groups[3].1[0].0, 4);
}

/// The panel's row actions reuse the tab lifecycle: switching and
/// closing still move `active` correctly, so a test that drives the
/// underlying methods is enough to pin the behaviour the panel
/// depends on.
#[gpui::test]
async fn file_panel_navigation_uses_tab_lifecycle(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    let (_dir, archive) = make_zip_archive(&["a.csv", "b.csv"]);
    let ar = archive.clone();
    cx.update_window_entity(&view, move |view, _, cx| {
        view.install_archive_tabs(ar, None, None, cx);
    });
    cx.run_until_parked();

    // Switch via the same path the panel's row click uses.
    cx.update_window_entity(&view, |view, _, cx| {
        view.switch_to(1, cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.active, Some(1));
        assert!(view.documents[1].is_loaded());
    });

    // Close via the same path the panel's × uses.
    cx.update_window_entity(&view, |view, _, cx| {
        view.close_active_at(1, cx);
    });
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.documents.len(), 1);
        assert_eq!(view.active, Some(0));
    });
}

// ---- filter ------------------------------------------------------

/// A small two-column fixture with headers, used by the filter
/// tests. Rows: (US,10), (UK,20), (US,30).
fn country_document() -> anvil_core::ParsedDocument {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("country.csv");
    std::fs::write(&path, b"country,amount\nUS,10\nUK,20\nUS,30\n").expect("write");
    let opened = anvil_core::source::local::open_local(&path).expect("open");
    anvil_core::index_opened(opened)
}

/// Cmd/Ctrl+L opens the filter bar through the grid keyboard path.
#[gpui::test]
async fn cmd_l_opens_filter_bar(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.filter_open);
    });
    let chord = if cfg!(target_os = "macos") {
        "cmd-l"
    } else {
        "ctrl-l"
    };
    cx.simulate_keystrokes(chord);
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.filter_open, "Cmd/Ctrl+L should open the bar");
    });
}

/// Cmd/Ctrl+Shift+L opens the Ask bar (not the plain filter bar).
#[gpui::test]
async fn cmd_shift_l_opens_ask_bar(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    let chord = if cfg!(target_os = "macos") {
        "cmd-shift-l"
    } else {
        "ctrl-shift-l"
    };
    cx.simulate_keystrokes(chord);
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.ask_open, "Cmd/Ctrl+Shift+L should open Ask");
        assert!(!view.filter_open, "the plain filter bar stays closed");
    });
}

/// Cmd/Ctrl+Shift+L while the filter bar is focused switches to Ask.
#[gpui::test]
async fn cmd_shift_l_switches_from_filter_to_ask(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.run_until_parked();
    let chord = if cfg!(target_os = "macos") {
        "cmd-shift-l"
    } else {
        "ctrl-shift-l"
    };
    cx.simulate_keystrokes(chord);
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.ask_open, "Ask should open");
        assert!(!view.filter_open, "the filter bar should close");
    });
}

/// A clean phrase previews and, on Enter, is written into the filter bar
/// and applied through the normal path.
#[gpui::test]
async fn ask_accepts_a_phrase_into_the_filter(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_ask(cx);
        view.ask_input = "country is US and amount over 25".into();
        view.refresh_ask();
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        let result = view.ask_result.as_ref().expect("ask result");
        assert!(result.clean, "{result:?}");
        assert_eq!(view.ask_status_text().0, "country == US AND amount > 25");
    });

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.ask_open, "Ask closes after accepting");
        assert!(view.filter_open, "the filter bar opens with the expression");
        assert_eq!(view.filter_input.text(), "country == US AND amount > 25");
        let rows = view.filter_rows.as_ref().expect("filter rows");
        assert_eq!(&**rows, &[2]);
    });
}

/// An unparseable phrase previews the failure and Enter does not apply
/// anything.
#[gpui::test]
async fn ask_does_not_apply_an_unparseable_phrase(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_ask(cx);
        view.ask_input = "banana".into();
        view.refresh_ask();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.ask_open, "Ask stays open on a failure");
        assert!(view.filter_rows.is_none());
    });
}

/// When the rules cannot resolve a phrase, Ctrl/Cmd+Enter escalates to the
/// injected Tier 2 interpreter and the validated result becomes the
/// preview.
#[gpui::test]
async fn ask_escalates_to_tier2_when_rules_are_unclean(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.ask_interpreter = Some(std::sync::Arc::new(|_phrase, _schema, _values| {
            Ok(anvil_core::FilterExpr::Predicate(anvil_core::Filter {
                column: 0,
                op: anvil_core::FilterOp::Eq,
                value: "US".to_string(),
            }))
        }));
        view.open_ask(cx);
        view.ask_input = "show me the american rows".into();
        view.refresh_ask();
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(
            !view.ask_result.as_ref().expect("result").clean,
            "Tier 1 should not resolve this phrase"
        );
    });

    cx.update_window_entity(&view, |view, _, cx| view.kick_ask_embedded(cx));
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        let result = view.ask_result.as_ref().expect("result");
        assert!(result.clean && result.from_model, "{result:?}");
        assert_eq!(
            result.expr,
            Some(anvil_core::FilterExpr::Predicate(anvil_core::Filter {
                column: 0,
                op: anvil_core::FilterOp::Eq,
                value: "US".to_string(),
            }))
        );
    });
}

/// Esc closes the Ask bar without touching the filter.
#[gpui::test]
async fn escape_closes_ask_bar(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, cx| view.open_ask(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.ask_open);
    });
}

/// Cmd/Ctrl+L while the search bar is focused switches to the filter
/// bar in one step (no need to close search first).
#[gpui::test]
async fn cmd_l_switches_from_search_bar_to_filter(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
    cx.run_until_parked();
    let chord = if cfg!(target_os = "macos") {
        "cmd-l"
    } else {
        "ctrl-l"
    };
    cx.simulate_keystrokes(chord);
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.search_open, "search bar should close");
        assert!(view.filter_open, "filter bar should open");
    });
}

/// Cmd/Ctrl+F while the filter bar is focused switches to search.
#[gpui::test]
async fn cmd_f_switches_from_filter_bar_to_search(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.run_until_parked();
    let chord = if cfg!(target_os = "macos") {
        "cmd-f"
    } else {
        "ctrl-f"
    };
    cx.simulate_keystrokes(chord);
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.filter_open, "filter bar should close");
        assert!(view.search_open, "search bar should open");
    });
}

/// The switch also works through the grid's action path: search open with
/// focus returned to the grid, Cmd/Ctrl+L switches to the filter bar.
#[gpui::test]
async fn cmd_l_from_grid_switches_open_search_to_filter(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject(cx, &view);
    cx.update_window_entity(&view, |view, _, cx| view.open_search(cx));
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, window, _| window.focus(&view.focus_handle));
    cx.run_until_parked();
    let chord = if cfg!(target_os = "macos") {
        "cmd-l"
    } else {
        "ctrl-l"
    };
    cx.simulate_keystrokes(chord);
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.search_open);
        assert!(view.filter_open);
    });
}

/// Keyboard editing in the filter field: End / Home move the caret
/// and Shift+Home extends the selection from the caret.
#[gpui::test]
async fn filter_keyboard_caret_and_selection(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_filter(cx);
        view.filter_input = "country == US".into();
    });
    cx.run_until_parked();
    let len = "country == US".len();

    cx.simulate_keystrokes("home");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_input.caret(), 0);
    });
    cx.simulate_keystrokes("end");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_input.caret(), len);
    });
    cx.simulate_keystrokes("left");
    cx.simulate_keystrokes("shift-home");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_input.selection(), 0..len - 1);
        assert_eq!(view.filter_input.selected_text(), "country == U");
    });
}

/// Cmd/Ctrl+A selects the whole field and typing replaces it;
/// Backspace removes the selection.
#[gpui::test]
async fn filter_select_all_typing_and_backspace(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_filter(cx);
        view.filter_input = "abc".into();
    });
    cx.run_until_parked();

    let select_all = if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    };
    cx.simulate_keystrokes(select_all);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_input.selected_text(), "abc");
    });
    cx.simulate_keystrokes("x");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_input.text(), "x");
        assert_eq!(view.filter_input.caret(), 1);
    });

    // Select all again and delete via Backspace.
    cx.simulate_keystrokes(select_all);
    cx.simulate_keystrokes("backspace");
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.filter_input.is_empty());
        assert_eq!(view.filter_input.caret(), 0);
    });
}

/// Mouse interaction: clicking positions the caret and a click-drag
/// selects the spanned range.
#[gpui::test]
async fn filter_mouse_click_and_drag(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_filter(cx);
        view.filter_input = "country == US".into();
    });
    cx.run_until_parked();
    let len = "country == US".len();

    let bounds = cx
        .debug_bounds("anvil-filter-input")
        .expect("filter input bounds");
    let y = bounds.origin.y + bounds.size.height / 2.0;
    let left = gpui::point(bounds.origin.x + px(1.0), y);
    let right = gpui::point(bounds.origin.x + bounds.size.width - px(1.0), y);
    let mid = gpui::point(bounds.origin.x + bounds.size.width / 2.0, y);
    let modifiers = gpui::Modifiers::default();

    // Click near the right edge: caret goes to the end of the text.
    cx.simulate_mouse_down(right, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(right, gpui::MouseButton::Left, modifiers);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.filter_input.caret(), len);
    });

    // Drag from the start towards the middle selects a prefix.
    cx.simulate_mouse_down(left, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_move(mid, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(mid, gpui::MouseButton::Left, modifiers);
    cx.update_window_entity(&view, |view, _, _| {
        let selection = view.filter_input.selection();
        assert_eq!(selection.start, 0);
        assert!(
            selection.end > 0 && selection.end <= len,
            "drag should select a non-empty prefix, got {selection:?}"
        );
    });
}

/// Enter parses the input and, on success, populates `filter_rows`
/// from the background evaluation. The fixture's `country == US`
/// rows are 0 and 2.
#[gpui::test]
async fn filter_input_applies_on_enter(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.update_window_entity(&view, |view, _, cx| {
        view.filter_input = "country == US".into();
        view.apply_filter_input(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.filter_applied.is_some());
        assert!(view.filter_error.is_none());
        let rows = view.filter_rows.as_ref().expect("filter rows");
        assert_eq!(&**rows, &[0, 2]);
    });
}

/// Compound predicates reach the data side through the filter bar.
/// Fixture rows are (US,10), (UK,20), (US,30): `AND` keeps only row 2,
/// `OR` keeps rows 1 and 2.
#[gpui::test]
async fn filter_input_applies_compound_expression_on_enter(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.update_window_entity(&view, |view, _, cx| {
        view.filter_input = "country == US AND amount > 25".into();
        view.apply_filter_input(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.filter_error.is_none());
        let rows = view.filter_rows.as_ref().expect("filter rows");
        assert_eq!(&**rows, &[2]);
    });

    cx.update_window_entity(&view, |view, _, cx| {
        view.filter_input = "(amount > 25 OR country == UK)".into();
        view.apply_filter_input(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.filter_error.is_none());
        let rows = view.filter_rows.as_ref().expect("filter rows");
        assert_eq!(&**rows, &[1, 2]);
    });
}

/// A parse error is shown and leaves the previous predicate in
/// place; no rows are installed.
#[gpui::test]
async fn filter_parse_error_is_shown(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.update_window_entity(&view, |view, _, cx| {
        view.filter_input = "nonsense".into();
        view.apply_filter_input(cx);
    });
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.filter_error.is_some());
        assert!(view.filter_applied.is_none());
        assert!(view.filter_rows.is_none());
    });
}

/// Clearing the filter restores the unfiltered document.
#[gpui::test]
async fn clear_filter_restores_document(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.update_window_entity(&view, |view, _, cx| {
        view.filter_input = "country == US".into();
        view.apply_filter_input(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.filter_rows.is_some());
    });

    cx.update_window_entity(&view, |view, _, cx| view.clear_filter(cx));
    cx.update_window_entity(&view, |view, _, _| {
        assert!(view.filter_applied.is_none());
        assert!(view.filter_rows.is_none());
        assert!(view.filter_input.is_empty());
    });
}

/// With a filter active, Down moves to the next *visible* record,
/// skipping hidden ones. Rows are (US,10), (UK,20), (US,30) and the
/// filter keeps 0 and 2, so Down from record 0 lands on record 2.
#[gpui::test]
async fn filter_vertical_navigation_skips_hidden_rows(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.update_window_entity(&view, |view, _, cx| {
        view.filter_input = "country == US".into();
        view.apply_filter_input(cx);
    });
    cx.run_until_parked();
    // The filter bar holds focus while open, so close it (which
    // returns focus to the grid) before driving arrow keys.
    cx.update_window_entity(&view, |view, _, cx| view.close_filter(cx));
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        view.selection.set(CellPos::new(0, 0));
    });

    cx.simulate_keystrokes("down");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.selection.cursor(), Some(CellPos::new(2, 0)));
    });

    // Up from the last visible row goes back to the first.
    cx.simulate_keystrokes("up");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.selection.cursor(), Some(CellPos::new(0, 0)));
    });
}

/// With a filter active, Tab / Shift-Tab and Cmd/Ctrl+End operate
/// on the *visible* rows. Rows are (US,10), (UK,20), (US,30); the
/// filter keeps records 0 and 2, so the visible grid is 2x2.
#[gpui::test]
async fn filter_tab_and_row_jumps_walk_visible_rows(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.update_window_entity(&view, |view, _, cx| {
        view.filter_input = "country == US".into();
        view.apply_filter_input(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, cx| view.close_filter(cx));
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        view.selection.set(CellPos::new(0, 0));
    });

    let end = if cfg!(target_os = "macos") {
        "cmd-end"
    } else {
        "ctrl-end"
    };
    // Cmd/Ctrl+End jumps to the last visible record, not the last
    // record in the file (which is filtered out).
    cx.simulate_keystrokes(end);
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.selection.cursor(), Some(CellPos::new(2, 0)));
    });

    // Tab wraps from the last visible cell back to the first.
    cx.simulate_keystrokes("tab");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.selection.cursor(), Some(CellPos::new(2, 1)));
    });
    cx.simulate_keystrokes("tab");
    cx.update_window_entity(&view, |view, _, _| {
        assert_eq!(view.selection.cursor(), Some(CellPos::new(0, 0)));
    });
}

/// The status label reports the filtered count and the original
/// total when a filter is active.
#[gpui::test]
async fn filter_status_label_reports_counts(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());

    cx.update_window_entity(&view, |view, _, cx| view.open_filter(cx));
    cx.update_window_entity(&view, |view, _, cx| {
        view.filter_input = "country == US".into();
        view.apply_filter_input(cx);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        let (text, _) = view.filter_status_text();
        assert!(text.contains("2 rows"), "got {text:?}");
    });
}

#[gpui::test]
async fn ask_does_not_accept_partial_preview(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_ask(cx);
        view.ask_result = Some(NlResult {
            expr: Some(FilterExpr::Predicate(anvil_core::Filter {
                column: 0,
                op: anvil_core::FilterOp::Eq,
                value: "US".into(),
            })),
            unmatched: vec!["unknown condition".into()],
            clean: false,
            from_model: false,
        });
        view.accept_ask(cx);
        assert!(view.ask_open);
        assert!(view.filter_rows.is_none());
        assert!(view.filter_input.text().is_empty());
    });
}

#[gpui::test]
async fn ask_edit_invalidates_inflight_model_result(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.ask_interpreter = Some(Arc::new(|_, _, _| {
            Ok(FilterExpr::Predicate(anvil_core::Filter {
                column: 0,
                op: anvil_core::FilterOp::Eq,
                value: "US".into(),
            }))
        }));
        view.open_ask(cx);
        view.ask_input = "unresolved request".into();
        view.refresh_ask();
        view.kick_ask_embedded(cx);
        let generation = view.ask_generation;
        view.ask_input = "country is UK".into();
        view.refresh_ask();
        assert_ne!(view.ask_generation, generation);
        assert!(!view.ask_loading);
    });
    cx.run_until_parked();
    cx.update_window_entity(&view, |view, _, _| {
        assert!(!view.ask_result.as_ref().unwrap().from_model);
        assert!(!view.ask_status_text().0.contains("US"));
    });
}

#[gpui::test]
async fn ask_document_change_clears_pending_preview(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| MainView::new(cx));
    inject_doc(cx, &view, country_document());
    cx.update_window_entity(&view, |view, _, cx| {
        view.open_ask(cx);
        let generation = view.ask_generation;
        view.ask_loading = true;
        view.load_active();
        assert_ne!(generation, view.ask_generation);
        assert!(!view.ask_open && !view.ask_loading);
        assert!(view.ask_schema.is_none() && view.ask_result.is_none());
    });
}

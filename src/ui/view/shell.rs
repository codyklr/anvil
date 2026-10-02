//! Window chrome and commands for the document investigation workspace.
//!
//! ## Why commands live outside the title bar
//!
//! `gpui_component::TitleBar` marks its entire content region
//! `WindowControlArea::Drag`. On Windows that region is reported to
//! `WM_NCHITTEST` as `HTCAPTION`, so the OS consumes every press there as a
//! window drag and the press never reaches the app. Buttons placed inside a
//! Kit `TitleBar` are therefore *dead on Windows* - hover works, clicks do
//! not. Anvil's title bar is consequently passive: wordmark and the active
//! document's identity only. Every command lives in
//! [`MainView::render_command_bar`], which is ordinary application content
//! and receives clicks on every platform.

use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
    ActiveTheme, Disableable, Icon, Selectable, Sizable, Theme, ThemeMode, TitleBar, WindowExt,
};
use gpui_kit::Div;

pub(super) fn tool(id: &'static str, label: &'static str, icon: IconName) -> Button {
    Button::new(id).ghost().small().icon(icon).label(label)
}

/// A separator that reads as structure rather than as a boxed control.
fn rule(colors: theme::Palette) -> Div {
    div().w(px(1.)).h(px(16.)).mx_1().bg(colors.border)
}

impl MainView {
    pub(crate) fn startup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        if let Some(path) = std::env::args().nth(1).filter(|arg| !arg.starts_with('-')) {
            self.begin_open(PathBuf::from(path), cx);
        }
    }

    pub(super) fn on_shell_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.secondary()
            && !event.keystroke.modifiers.alt
            && (event.keystroke.key == "?"
                || (event.keystroke.key == "/" && event.keystroke.modifiers.shift))
        {
            Self::show_shortcuts(window, cx);
            cx.stop_propagation();
            return;
        }
        let action = commands::action_for(&event.keystroke);
        if matches!(action, Some(Action::OpenFile)) {
            self.pick_file(cx);
            cx.stop_propagation();
        } else if self.show_picker && event.keystroke.key == "escape" {
            self.show_picker = false;
            self.focus_grid_on_next_frame = true;
            cx.notify();
            cx.stop_propagation();
        } else if !self.focus_handle.is_focused(window)
            && !self.search_focus_handle.is_focused(window)
            && !self.filter_focus_handle.is_focused(window)
            && !self.ask_focus_handle.is_focused(window)
            && !self.goto_focus_handle.is_focused(window)
            && matches!(
                action,
                Some(
                    Action::OpenSearch
                        | Action::OpenFilter
                        | Action::OpenAsk
                        | Action::ToggleFilePanel
                        | Action::ToggleInspector
                        | Action::ToggleDiagnostics
                        | Action::OpenGoToRow
                        | Action::RevealCell
                )
            )
        {
            self.run_action(action.unwrap(), cx);
            cx.stop_propagation();
        }
    }

    /// Passive window title bar: identity only.
    ///
    /// See the module note - no interactive control may be placed here or it
    /// will not receive clicks on Windows.
    pub(super) fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme::palette(cx);
        let (name, sub) = self.window_identity();
        TitleBar::new()
            .bg(colors.chrome)
            // The halftone layer of the menu chrome. It is a GPU
            // pattern, so it repaints for free; it carries no
            // handlers and never intercepts a click or a drag.
            .child(texture::chrome_dither(colors.foreground.alpha(0.025)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(colors.strong_foreground)
                            .child("ANVIL"),
                    )
                    .when(!name.is_empty(), |this| {
                        this.child(div().w(px(1.)).h(px(10.)).bg(colors.border_strong))
                            .child(
                                div()
                                    .flex()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .truncate()
                                    .child(sub),
                            )
                    }),
            )
    }

    /// Wordmark plus the active document's identity, split out so the title
    /// bar and the empty state agree on what the window is showing.
    fn window_identity(&self) -> (String, String) {
        match self
            .active
            .and_then(|ix| self.documents.get(ix))
            .map(DocumentSnapshot::tab_label)
        {
            Some(label) => ("document".into(), label),
            None => (String::new(), String::new()),
        }
    }

    /// The persistent command bar.
    ///
    /// Left: the actions a user reaches for while reading rows. Right: the
    /// surfaces that change what the workspace shows. Open is the only
    /// primary-styled control; everything else is a quiet toggle so the bar
    /// never competes with the data below it.
    pub(super) fn render_command_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme::palette(cx);
        let ready = self.model.is_some();
        let entity = cx.entity().downgrade();
        let headers = header_mode_label(self.header_mode);
        let archive = self.archive.is_some();
        let has_documents = !self.documents.is_empty();

        div()
            .flex()
            .items_center()
            .gap_1()
            .h(theme::COMMAND_BAR_HEIGHT)
            .px_2()
            .flex_shrink_0()
            .bg(colors.chrome)
            .border_b_1()
            .border_color(colors.border)
            // The halftone layer of the menu chrome. A GPU pattern,
            // so it repaints for free; no handlers, so it never
            // intercepts a click.
            .child(texture::chrome_dither(colors.foreground.alpha(0.025)))
            .child(
                Button::new("open-button")
                    .small()
                    .icon(IconName::FolderOpen)
                    .label("Open")
                    .tooltip("Open a data file or archive · Ctrl/Cmd+O")
                    .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx))),
            )
            .child(rule(colors))
            .child(
                tool("find", "Find", IconName::Search)
                    .selected(self.search_open)
                    .disabled(!ready)
                    .tooltip("Find in data · Ctrl/Cmd+F")
                    .on_click(cx.listener(|this, _, _, cx| this.switch_to_search(cx))),
            )
            .child(
                tool("filter", "Filter", IconName::ListFilter)
                    .selected(self.filter_open || self.filter_applied.is_some())
                    .disabled(!ready)
                    .tooltip("Filter rows · Ctrl/Cmd+L")
                    .on_click(cx.listener(|this, _, _, cx| this.switch_to_filter(cx))),
            )
            .child(
                tool("ask", "Ask", IconName::Sparkles)
                    .selected(self.ask_open)
                    .disabled(!ready)
                    .tooltip("Ask in plain language · Ctrl/Cmd+Shift+L")
                    .on_click(cx.listener(|this, _, _, cx| this.switch_to_ask(cx))),
            )
            .child(div().flex_1().min_w_0())
            // ---- right-hand workspace toggles ----
            .child(
                tool("toggle-files", "Files", IconName::PanelLeft)
                    .selected(self.file_panel_open)
                    .disabled(!has_documents)
                    .tooltip("Show open files · Ctrl/Cmd+B")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.file_panel_open = !this.file_panel_open;
                        cx.notify();
                    })),
            )
            .child(
                tool("toggle-inspector", "Columns", IconName::PanelRight)
                    .selected(self.inspector_open)
                    .disabled(!ready)
                    .tooltip("Inspect a column · Ctrl/Cmd+I")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_inspector(cx))),
            )
            .child(
                tool("toggle-diagnostics", "Diagnostics", IconName::TriangleAlert)
                    .selected(self.diagnostics_open)
                    .disabled(!ready)
                    .tooltip("Inspect data quality · Ctrl/Cmd+D")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_diagnostics(cx))),
            )
            .child(rule(colors))
            .child(
                Button::new("appearance")
                    .ghost()
                    .small()
                    .icon(if cx.theme().mode == ThemeMode::Dark {
                        IconName::Sun
                    } else {
                        IconName::Moon
                    })
                    .tooltip("Switch light / dark appearance")
                    .on_click(|_, window, cx| {
                        let mode = if cx.theme().mode == ThemeMode::Dark {
                            ThemeMode::Light
                        } else {
                            ThemeMode::Dark
                        };
                        Theme::change(mode, Some(window), cx);
                    }),
            )
            .child(
                Button::new("shortcuts")
                    .ghost()
                    .small()
                    .icon(IconName::Keyboard)
                    .tooltip("Keyboard shortcuts · Ctrl/Cmd+?")
                    .on_click(|_, window, cx| {
                        Self::show_shortcuts(window, cx);
                    }),
            )
            .child(
                Button::new("document-menu")
                    .ghost()
                    .small()
                    .icon(IconName::Ellipsis)
                    .tooltip("Document commands")
                    .dropdown_menu(move |menu, _, _| {
                        let go = entity.clone();
                        let reveal = entity.clone();
                        let header = entity.clone();
                        let archive_view = entity.clone();
                        menu.item(
                            PopupMenuItem::new("Go to row…")
                                .icon(IconName::ArrowDown)
                                .disabled(!ready)
                                .on_click(move |_, _, cx| {
                                    let _ = go.update(cx, |view, cx| view.open_goto(cx));
                                }),
                        )
                        .item(
                            PopupMenuItem::new("Reveal cell")
                                .icon(IconName::Square)
                                .disabled(!ready)
                                .on_click(move |_, _, cx| {
                                    let _ =
                                        reveal.update(cx, |view, cx| view.toggle_cell_detail(cx));
                                }),
                        )
                        .separator()
                        .item(PopupMenuItem::new(headers).disabled(!ready).on_click(
                            move |_, _, cx| {
                                let _ = header.update(cx, |view, cx| view.cycle_header_mode(cx));
                            },
                        ))
                        .item(
                            PopupMenuItem::new("Browse archive…")
                                .icon(IconName::FolderOpen)
                                .disabled(!archive)
                                .on_click(move |_, _, cx| {
                                    let _ = archive_view.update(cx, |view, cx| {
                                        view.open_archive_picker(cx);
                                    });
                                }),
                        )
                    }),
            )
    }

    pub(super) fn show_shortcuts(window: &mut Window, cx: &mut gpui_kit::App) {
        window.open_dialog(cx, |dialog, _, cx| {
            let colors = theme::palette(cx);
            let modifier = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+"
            };
            let groups: [(&str, &[(&str, &str)]); 3] = [
                (
                    "Documents",
                    &[
                        ("Open file", "O"),
                        ("Files panel", "B"),
                        ("Go to row", "G"),
                        ("Reveal cell", "Enter"),
                    ],
                ),
                (
                    "Queries",
                    &[
                        ("Find", "F"),
                        ("Find next / previous", "F3 / ⇧F3"),
                        ("Filter", "L"),
                        ("Ask", "⇧L"),
                    ],
                ),
                (
                    "View",
                    &[
                        ("Column inspector", "I"),
                        ("Diagnostics", "D"),
                        ("Copy selection", "C"),
                        ("Select all", "A"),
                    ],
                ),
            ];
            let mut columns: Vec<Div> = Vec::new();
            for (title, rows) in groups {
                columns.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(colors.muted_foreground)
                                .child(title.to_uppercase()),
                        )
                        .children(rows.iter().map(|(label, key)| {
                            let key = if key.starts_with(char::is_alphabetic)
                                && key.chars().count() <= 2
                                && !key.contains('F')
                            {
                                format!("{modifier}{key}")
                            } else {
                                (*key).to_string()
                            };
                            div()
                                .flex()
                                .justify_between()
                                .gap_4()
                                .text_sm()
                                .child(*label)
                                .child(
                                    div()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(colors.muted_foreground)
                                        .child(key),
                                )
                        })),
                );
            }
            dialog.title("Keyboard shortcuts").child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .text_sm()
                    .children(columns)
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.faint_foreground)
                            .child("Arrows move · Shift extends · Tab advances · Escape dismisses"),
                    ),
            )
        });
    }

    pub(super) fn open_archive_picker(&mut self, cx: &mut Context<Self>) {
        self.show_picker = true;
        cx.notify();
    }

    /// The "no file open" state.
    ///
    /// This is a real product state, not a landing page: it says what Anvil
    /// opens, offers the one action that resolves it, and reports the
    /// keyboard route. The dithered field behind it is the same atmosphere
    /// the command bar sits in, so the empty window still reads as part of
    /// the product rather than as a placeholder.
    pub(super) fn render_welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme::palette(cx);
        div()
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .items_center()
            .justify_center()
            .px(px(32.))
            .py(px(24.))
            .bg(colors.canvas)
            // Keep the entrance quiet; the bounded ordered field carries
            // the material identity without texturing the whole workspace.
            .child(texture::home_field(colors.primary))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .max_w(px(880.))
                    .items_start()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .w_full()
                            .max_w(px(400.))
                            .px(px(24.))
                            .py(px(24.))
                            .bg(colors.canvas)
                            .drag_over::<gpui_kit::ExternalPaths>(move |style, _, _, _| {
                                style
                                    .bg(colors.chrome_active)
                                    .border_1()
                                    .border_color(colors.primary)
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_start()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(32.))
                                            .child(
                                                Icon::new(IconName::Table)
                                                    .size_6()
                                                    .text_color(colors.muted_foreground),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(24.))
                                            .line_height(px(30.))
                                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                                            .text_color(colors.strong_foreground)
                                            .child("Open a data file"),
                                    ),
                            )
                            .child(
                                div()
                                    .mt(px(8.))
                                    .max_w(px(360.))
                                    .text_size(px(13.))
                                    .line_height(px(20.))
                                    .text_color(colors.muted_foreground)
                                    .child("Drop a CSV, TSV or ZIP anywhere in this window."),
                            )
                            .child(
                                div().flex().items_center().mt(px(20.)).child(
                                    Button::new("welcome-open")
                                        .primary()
                                        .small()
                                        .min_w(px(120.))
                                        .min_h(px(32.))
                                        .label("Open file")
                                        .icon(IconName::FolderOpen)
                                        .tooltip(format!(
                                            "Open a data file or archive · {}",
                                            shortcut_label("Ctrl+O")
                                        ))
                                        .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx))),
                                ),
                            )
                            .child(
                                div().flex().items_center().mt(px(8.)).child(
                                    div()
                                        .text_size(px(12.))
                                        .line_height(px(16.))
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(colors.muted_foreground)
                                        .child(shortcut_label("Ctrl+O")),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// Platform-correct spelling of a keyboard chord, for inline hints.
pub(super) fn shortcut_label(chord: &str) -> String {
    if cfg!(target_os = "macos") {
        chord.replace("Ctrl+", "⌘")
    } else {
        chord.to_string()
    }
}

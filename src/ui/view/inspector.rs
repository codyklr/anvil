//! Column inspector panel: state, background stats scan, and render.
//!
//! The stats themselves come from `anvil_core::schema::stats`; this
//! module is the view-side lifecycle. Methods moved verbatim out of
//! `view.rs` (Iteration 10 refactor, no behaviour change).

use super::*;

impl MainView {
    /// Right-side column-inspector panel. Shows the selected column's
    /// inferred type and metrics, or a hint when nothing is selected.
    pub(super) fn render_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
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

        // Make it obvious what the numbers describe when a filter or sort
        // is active; the scan is scoped to the visible rows then.
        let scope_label = match self.view_rows() {
            Some(rows) => format!(
                "scoped to current view ({} of {} rows)",
                format_count(rows.len() as u64),
                format_count(self.model.as_ref().map(|m| m.row_count()).unwrap_or(0))
            ),
            None => "whole document".to_string(),
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
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(theme::TEXT_DIM)
                    .child(scope_label),
            )
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

    /// Show or hide the column-inspector panel. Opening it kicks a stats
    /// scan for the currently selected column.
    pub(super) fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_open = !self.inspector_open;
        if self.inspector_open {
            self.inspector_column = None;
            self.inspector_stats = None;
            self.kick_inspector(cx);
        }
        cx.notify();
    }

    /// Recompute the selected column's stats when the inspector is open.
    /// No-op when the column has not changed since the last scan, so
    /// vertical navigation does not rescan. Runs on the background
    /// executor with a generation guard, like the filter/sort tasks.
    pub(super) fn kick_inspector(&mut self, cx: &mut Context<Self>) {
        // The full-cell panel follows the same cursor; refresh it first so
        // both panels update together. `kick_cell_detail` is a no-op when
        // the panel is closed.
        self.kick_cell_detail(cx);
        if !self.inspector_open {
            return;
        }
        let Some(column) = self.selection.cursor().map(|c| c.column_index) else {
            return;
        };
        // Rescan when the column changes or when a filter/sort change has
        // moved the view under the open inspector.
        let view_key = (self.filter_generation, self.sort_generation);
        if self.inspector_column == Some(column)
            && self.inspector_stats.is_some()
            && self.inspector_view_key == view_key
        {
            return;
        }
        let Some(doc) = self.model.as_ref().map(|m| m.doc.clone()) else {
            return;
        };
        let view_rows = self.view_rows();
        self.inspector_column = Some(column);
        self.inspector_view_key = view_key;
        self.inspector_generation = self.inspector_generation.wrapping_add(1);
        let generation = self.inspector_generation;
        self.inspector_stats = None;

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let stats = executor
                .spawn(async move {
                    // A filtered/sorted view is scanned on its own; only
                    // the visible rows are counted, measured and sampled.
                    match &view_rows {
                        Some(rows) => {
                            column_stats_rows(&doc, column, rows, &StatsOptions::default())
                        }
                        None => column_stats(&doc, column, &StatsOptions::default()),
                    }
                })
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

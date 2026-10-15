//! Column inspector panel: state, background stats scan, and render.
//!
//! The stats themselves come from `anvil_core::schema::stats`; this
//! module is the view-side lifecycle. Methods moved verbatim out of
//! `view.rs` (Iteration 10 refactor, no behaviour change).

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, IconName, Sizable};

use super::*;

impl MainView {
    /// Right-side column-inspector panel: the selected column's inferred
    /// type, counts, exact extremes, samples and frequent values.
    pub(super) fn render_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme::palette(cx);
        let header = panel::header(
            colors,
            IconName::Inspector,
            "Column",
            Button::new("inspector-close")
                .xsmall()
                .ghost()
                .icon(IconName::Close)
                .accessibility_label("Close column inspector")
                .tooltip("Close column inspector")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_inspector(cx))),
        );

        // Make it obvious what the numbers describe when a filter or sort is
        // active; the scan is scoped to the visible rows then.
        let scope = match self.view_rows() {
            Some(rows) => format!(
                "Filtered view · {} of {} rows",
                format_count(rows.len() as u64),
                format_count(self.model.as_ref().map(|m| m.row_count()).unwrap_or(0))
            ),
            None => "Whole document".to_string(),
        };

        let content: Vec<AnyElement> = match &self.inspector_stats {
            Some(stats) => column_stats_content(cx, stats, &scope),
            None if self.model.is_none() => vec![panel::empty_state(
                colors,
                "No document open",
                "Open a data file to inspect its columns.",
            )],
            None if self.inspector_column.is_some() => vec![panel::empty_state(
                colors,
                "Scanning values",
                "Reading the column in the background. The grid stays usable.",
            )],
            None => vec![panel::empty_state(
                colors,
                "No column selected",
                "Select a cell to see its type, counts and values.",
            )],
        };

        div()
            .w_full()
            .min_w_0()
            .h_full()
            .font_family(cx.theme().font_family.clone())
            .text_sm()
            .flex()
            .flex_col()
            .bg(colors.panel)
            .child(header)
            .child(panel::body("inspector-body", content))
            .into_any_element()
    }

    /// Show or hide the column-inspector panel. Opening it kicks a stats
    /// scan for the currently selected column.
    pub(super) fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_open = !self.inspector_open;
        if self.inspector_open {
            self.diagnostics_open = false;
            self.close_cell_detail(cx);
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
        let encoding = self.display_encoding;
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
                    let options = StatsOptions {
                        encoding,
                        ..StatsOptions::default()
                    };
                    match &view_rows {
                        Some(rows) => column_stats_rows(&doc, column, rows, &options),
                        None => column_stats(&doc, column, &options),
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

/// The inspector body for a computed column.
///
/// Ordered by what a reader checks first: what it is, how big it is, what
/// the extremes are, then examples. Exact values go in recessed wells so
/// they are never mistaken for the labels beside them.
fn column_stats_content(cx: &gpui_kit::App, stats: &ColumnStats, scope: &str) -> Vec<AnyElement> {
    let colors = theme::palette(cx);
    let rows = format_count(stats.total_rows);
    let distinct = match stats.distinct {
        DistinctCount::Exact(n) => format_count(n),
        DistinctCount::Approximate(n) => format!("~{}", format_count(n)),
    };

    let mut out: Vec<AnyElement> = vec![
        panel::title_block(cx, colors, stats.name.clone(), scope.to_string()).into_any_element(),
        panel::section(colors, "Type").into_any_element(),
        panel::metric(cx, colors, "Inferred", &format!("{:?}", stats.column_type))
            .into_any_element(),
        panel::section(colors, "Counts").into_any_element(),
        panel::metric(cx, colors, "Rows", &rows).into_any_element(),
        panel::metric(cx, colors, "Non-empty", &format_count(stats.non_empty)).into_any_element(),
        panel::metric(cx, colors, "Empty", &format_count(stats.empty)).into_any_element(),
        panel::metric(cx, colors, "Distinct", &distinct).into_any_element(),
        panel::metric(
            cx,
            colors,
            "Duplicates",
            &stats
                .duplicate_count
                .map(format_count)
                .unwrap_or_else(|| "Not available".to_string()),
        )
        .into_any_element(),
        panel::section(colors, "Extremes").into_any_element(),
        panel::data_well(
            cx,
            colors,
            "Minimum",
            stat_value(&stats.min),
            stats.min.is_some(),
        )
        .into_any_element(),
        panel::data_well(
            cx,
            colors,
            "Maximum",
            stat_value(&stats.max),
            stats.max.is_some(),
        )
        .into_any_element(),
        panel::data_well(
            cx,
            colors,
            "Longest",
            stat_value(&stats.longest),
            stats.longest.is_some(),
        )
        .into_any_element(),
        panel::data_well(
            cx,
            colors,
            "Shortest",
            stat_value(&stats.shortest),
            stats.shortest.is_some(),
        )
        .into_any_element(),
    ];

    out.push(panel::section(colors, "Samples").into_any_element());
    if stats.samples.is_empty() {
        out.push(panel::caption(colors, "No non-empty values in this scope").into_any_element());
    } else {
        for sample in &stats.samples {
            out.push(panel::data_well(cx, colors, "", sample, true).into_any_element());
        }
    }

    out.push(panel::section(colors, "Frequent values").into_any_element());
    if stats.frequent.is_empty() {
        out.push(
            panel::caption(
                colors,
                match stats.distinct {
                    DistinctCount::Approximate(_) => {
                        "Unavailable for an approximate distinct count"
                    }
                    DistinctCount::Exact(_) => "No non-empty values in this scope",
                },
            )
            .into_any_element(),
        );
    } else {
        // Ranked rows: the value is the subject, the count is the measure.
        for entry in &stats.frequent {
            out.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px(px(panel::BODY_PAD_X))
                    .py(px(2.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .font_family(panel::mono(cx))
                            .text_color(colors.foreground)
                            .truncate()
                            .child(entry.value.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .font_family(panel::mono(cx))
                            .text_color(colors.muted_foreground)
                            .child(format_count(entry.count)),
                    )
                    .into_any_element(),
            );
        }
    }

    out
}

/// The string behind an optional stat, or an empty string when absent.
fn stat_value(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("")
}

//! Column sorting for the active tab: cycle, background permutation,
//! and the status label. The permutation itself lives in
//! `anvil_core::query::sort`; this module is the view-side state machine.

use super::*;

impl MainView {
    /// The record indexes in display order for the active tab: the
    /// sorted permutation when a sort is active, otherwise the filtered
    /// set, otherwise `None` (identity order). Sorting composes with
    /// filtering because `sort_rows` is built from `filter_rows`.
    pub(super) fn view_rows(&self) -> Option<Arc<Vec<u64>>> {
        self.sort_rows.clone().or_else(|| self.filter_rows.clone())
    }

    /// Cycle the active tab's sort for `column`: none -> ascending ->
    /// descending -> none. Sorting a different column restarts at
    /// ascending. The numeric/text comparison kind is resolved from
    /// inferred column types when the background task runs.
    pub(super) fn cycle_sort(&mut self, column: usize, cx: &mut Context<Self>) {
        let next = match self.sort_spec {
            Some(spec) if spec.column == column && spec.order == SortOrder::Ascending => {
                Some(SortSpec {
                    column,
                    order: SortOrder::Descending,
                    kind: spec.kind,
                })
            }
            Some(spec) if spec.column == column && spec.order == SortOrder::Descending => None,
            _ => Some(SortSpec {
                column,
                order: SortOrder::Ascending,
                kind: SortKind::Text,
            }),
        };
        self.sort_spec = next;
        self.kick_sort(cx);
    }

    /// Recompute the active tab's sort permutation on the background
    /// executor. `None` spec clears it. A generation guard discards
    /// stale results, mirroring the filter task.
    pub(super) fn kick_sort(&mut self, cx: &mut Context<Self>) {
        let Some(spec) = self.sort_spec else {
            self.sort_rows = None;
            self.sort_generation = self.sort_generation.wrapping_add(1);
            self.sort_task = None;
            cx.notify();
            return;
        };
        let Some(doc) = self.model.as_ref().map(|m| m.doc.clone()) else {
            self.sort_rows = None;
            return;
        };
        // The base is whatever is currently visible: the filtered set,
        // or every record when there is no filter.
        let base: Vec<u64> = match &self.filter_rows {
            Some(rows) => (**rows).clone(),
            None => (0..doc.record_count()).collect(),
        };
        self.sort_generation = self.sort_generation.wrapping_add(1);
        let generation = self.sort_generation;
        // Show the unsorted base while the sort runs.
        self.sort_rows = None;

        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let sorted = executor
                .spawn(async move {
                    let profiles = infer_schema(&doc);
                    let kind = profiles
                        .get(spec.column)
                        .map(|profile| SortKind::for_column_type(profile.column_type))
                        .unwrap_or(SortKind::Text);
                    sort_rows(&doc, &base, &SortSpec { kind, ..spec })
                })
                .await;
            this.update(cx, |view, cx| {
                if view.sort_generation != generation {
                    return;
                }
                view.sort_rows = Some(Arc::new(sorted));
                // A scoped inspector must reflect the new visible set.
                view.kick_inspector(cx);
                cx.notify();
            })
            .ok();
        });
        self.sort_task = Some(task);
        cx.notify();
    }

    /// Status-bar label for the active sort (column name + direction).
    pub(super) fn sort_label(&self) -> Option<String> {
        let spec = self.sort_spec?;
        let name = self
            .model
            .as_ref()
            .map(|m| m.doc.meta.column_label(spec.column))
            .unwrap_or_else(|| format!("col{}", spec.column));
        let arrow = match spec.order {
            SortOrder::Ascending => "▲",
            SortOrder::Descending => "▼",
        };
        Some(format!("sorted by {name} {arrow}"))
    }
}

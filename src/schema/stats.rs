//! Column statistics for the column inspector.
//!
//! Scans one column and produces the metrics the inspector shows:
//! counts, distinct cardinality, min/max, longest/shortest, samples,
//! and the most frequent values. Every displayed value is the exact cell
//! value: padding is preserved and `" 42 "` is a different value from
//! `"42"`. Ordering (min/max) compares exact bytes for a text column
//! (case-sensitive, padding-preserving), so values differing only in
//! padding or case are never equal; numeric columns keep their numeric
//! comparison.
//!
//! A cell is empty only when it has **zero bytes**. A whitespace-only
//! cell (`"   "`) is a value, matching the filter `empty` operator
//! (`bytes.is_empty()`) and the byte-faithful rule in ADR 0003.
//!
//! Distinct counting is exact while the
//! value set stays under a threshold; past that it switches to a
//! HyperLogLog estimate so a high-cardinality column does not blow up
//! memory. This is GPUI-free and lives in `anvil_core`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use crate::parser::{ColumnIndex, ParsedDocument};

use super::{infer_column, ColumnType};

/// Distinct-value count, with its accuracy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistinctCount {
    Exact(u64),
    Approximate(u64),
}

/// A value and how many times it occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueCount {
    pub value: String,
    pub count: u64,
}

/// Everything the inspector displays for one column.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnStats {
    pub column: usize,
    pub name: String,
    pub column_type: ColumnType,
    pub total_rows: u64,
    pub non_empty: u64,
    pub empty: u64,
    pub distinct: DistinctCount,
    /// `non_empty - distinct` when the distinct count is exact.
    pub duplicate_count: Option<u64>,
    /// Smallest value by the column's ordering, exactly as it appears in
    /// the file (padding included).
    pub min: Option<String>,
    /// Largest value by the column's ordering, exactly as it appears in
    /// the file.
    pub max: Option<String>,
    /// The longest non-empty value, exactly as it appears in the file;
    /// the length is counted on that exact value.
    pub longest: Option<String>,
    /// The shortest non-empty value, exactly as it appears in the file.
    pub shortest: Option<String>,
    /// The first few non-empty values, in row order and exactly as they
    /// appear in the file.
    pub samples: Vec<String>,
    /// The `top_values` most frequent values (exact mode only), exactly as
    /// they appear in the file. Values that differ only in padding are
    /// counted separately, because they are different values.
    pub frequent: Vec<ValueCount>,
}

/// Tunables for [`stats`].
#[derive(Debug, Clone, Copy)]
pub struct StatsOptions {
    /// Above this many distinct values, switch to an HLL estimate.
    pub max_exact_distinct: usize,
    /// How many frequent values to return.
    pub top_values: usize,
    /// How many samples to return.
    pub samples: usize,
}

impl Default for StatsOptions {
    fn default() -> Self {
        Self {
            max_exact_distinct: 100_000,
            top_values: 10,
            samples: 8,
        }
    }
}

/// Compute the inspector metrics for `column`.
pub fn stats(doc: &ParsedDocument, column: usize, options: &StatsOptions) -> ColumnStats {
    let profile = infer_column(doc, column);
    let numeric = profile.column_type.is_numeric();
    let total_rows = doc.record_count();

    let mut non_empty = 0u64;
    let mut empty = 0u64;
    let mut counts: Option<HashMap<String, u64>> = Some(HashMap::new());
    let mut hll: Option<Hll> = None;
    let mut min: Option<String> = None;
    let mut max: Option<String> = None;
    let mut longest: Option<(String, usize)> = None;
    let mut shortest: Option<(String, usize)> = None;
    let mut samples: Vec<String> = Vec::with_capacity(options.samples);

    for row in 0..total_rows {
        let Some(cell) = doc.cell(row, ColumnIndex(column)) else {
            continue;
        };
        // `exact` is what the inspector displays: the value as it exists
        // in the file, padding included. A cell is empty only when it has
        // zero bytes; a whitespace-only cell is a value (ADR 0003).
        let exact = cell.to_exact_string();
        if exact.is_empty() {
            empty += 1;
            continue;
        }
        non_empty += 1;

        if samples.len() < options.samples {
            samples.push(exact.to_string());
        }

        match &min {
            None => min = Some(exact.to_string()),
            Some(current) => {
                if compare_values(&exact, current, numeric) == Ordering::Less {
                    min = Some(exact.to_string());
                }
            }
        }
        match &max {
            None => max = Some(exact.to_string()),
            Some(current) => {
                if compare_values(&exact, current, numeric) == Ordering::Greater {
                    max = Some(exact.to_string());
                }
            }
        }

        let char_len = exact.chars().count();
        let longer = match longest.as_ref() {
            None => true,
            Some((_, len)) => char_len > *len,
        };
        if longer {
            longest = Some((exact.to_string(), char_len));
        }
        let shorter = match shortest.as_ref() {
            None => true,
            Some((_, len)) => char_len < *len,
        };
        if shorter {
            shortest = Some((exact.to_string(), char_len));
        }

        let mut switch = false;
        if let Some(map) = counts.as_mut() {
            // Counted by the exact value, so `" 42 "` and `42` stay two
            // distinct values instead of being silently merged.
            *map.entry(exact.to_string()).or_insert(0) += 1;
            switch = map.len() > options.max_exact_distinct;
        }
        if switch {
            let mut sketch = Hll::new();
            if let Some(map) = counts.take() {
                for value in map.keys() {
                    sketch.add(value);
                }
            }
            hll = Some(sketch);
        } else if counts.is_none() {
            if let Some(sketch) = hll.as_mut() {
                sketch.add(&exact);
            }
        }
    }

    let distinct = match (&counts, &hll) {
        (Some(map), _) => DistinctCount::Exact(map.len() as u64),
        (None, Some(sketch)) => DistinctCount::Approximate(sketch.estimate()),
        _ => DistinctCount::Exact(0),
    };
    let duplicate_count = counts
        .as_ref()
        .map(|map| non_empty.saturating_sub(map.len() as u64));

    let mut frequent: Vec<ValueCount> = counts
        .as_ref()
        .map(|map| {
            map.iter()
                .map(|(value, count)| ValueCount {
                    value: value.clone(),
                    count: *count,
                })
                .collect()
        })
        .unwrap_or_default();
    frequent.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    frequent.truncate(options.top_values);

    ColumnStats {
        column,
        name: profile.name,
        column_type: profile.column_type,
        total_rows,
        non_empty,
        empty,
        distinct,
        duplicate_count,
        min,
        max,
        longest: longest.map(|(value, _)| value),
        shortest: shortest.map(|(value, _)| value),
        samples,
        frequent,
    }
}

/// Compare two non-empty values. Numeric columns compare numerically
/// (the parse tolerates surrounding whitespace, matching the old
/// behaviour); every other column compares exact bytes, case-sensitive
/// and padding-preserving. Non-numeric values in a numeric column compare
/// equal so they neither set nor dislodge min/max.
fn compare_values(a: &str, b: &str, numeric: bool) -> Ordering {
    if numeric {
        match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
            (Ok(x), Ok(y)) => x.total_cmp(&y),
            _ => Ordering::Equal,
        }
    } else {
        a.cmp(b)
    }
}

/// A tiny HyperLogLog sketch (1024 registers, ~3% standard error) used
/// once a column's cardinality passes `max_exact_distinct`.
struct Hll {
    registers: Vec<u8>,
}

impl Hll {
    const BITS: u32 = 10;
    const M: usize = 1 << Self::BITS;

    fn new() -> Self {
        Self {
            registers: vec![0; Self::M],
        }
    }

    fn add(&mut self, value: &str) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        let hash = hasher.finish();
        let index = (hash & (Self::M as u64 - 1)) as usize;
        let remainder = hash >> Self::BITS;
        // The top `BITS` bits are zero after the shift, so subtract them
        // before counting the rank of the first set bit.
        let rank = (remainder.leading_zeros() - Self::BITS + 1) as u8;
        let register = &mut self.registers[index];
        if rank > *register {
            *register = rank;
        }
    }

    fn estimate(&self) -> u64 {
        let m = Self::M as f64;
        let alpha = 0.7213 / (1.0 + 1.079 / m);
        let sum: f64 = self
            .registers
            .iter()
            .map(|&register| 2f64.powi(-(register as i32)))
            .sum();
        let raw = alpha * m * m / sum;
        // Small-range correction (linear counting): with few distinct
        // values most registers are empty and the raw estimate is far
        // too high.
        let zeros = self.registers.iter().filter(|&&r| r == 0).count() as f64;
        let estimate = if raw <= 2.5 * m && zeros > 0.0 {
            m * (m / zeros).ln()
        } else {
            raw
        };
        estimate.round() as u64
    }
}

/// Collect up to `max_distinct` distinct non-empty values for `column`,
/// visiting at most `max_rows` records spread evenly across the whole
/// document. Designed for filter autocomplete, so it is bounded and does
/// not materialise the whole column; values are returned in first-seen
/// order.
///
/// Sampling across the file (rather than only its head) matters when a
/// column is sorted: a rare value such as `Madagascar` in a country
/// column may first appear far past the first few thousand rows, and a
/// head-only scan would never suggest it even though the filter finds
/// it.
pub fn sample_distinct(
    doc: &ParsedDocument,
    column: usize,
    max_rows: usize,
    max_distinct: usize,
) -> Vec<String> {
    let total = doc.record_count();
    if total == 0 || max_rows == 0 || max_distinct == 0 {
        return Vec::new();
    }
    let budget = max_rows as u64;
    // Visit ~`max_rows` records evenly spaced over the document.
    let step = if total > budget {
        (total / budget).max(1)
    } else {
        1
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut row = 0u64;
    let mut visited = 0u64;
    while row < total && visited < budget && out.len() < max_distinct {
        if let Some(cell) = doc.cell(row, ColumnIndex(column)) {
            let value = cell.as_str().trim();
            if !value.is_empty() && seen.insert(value.to_string()) {
                out.push(value.to_string());
            }
        }
        row = row.saturating_add(step);
        visited += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_from(bytes: &[u8]) -> ParsedDocument {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("stats.csv");
        std::fs::write(&path, bytes).expect("write");
        let opened = crate::source::local::open_local(&path).expect("open");
        crate::index_opened(opened)
    }

    #[test]
    fn counts_empties_and_distinct() {
        let doc = doc_from(b"v,pad\na,a\nb,a\na,a\n,a\n,a\n");
        let stats = stats(&doc, 0, &StatsOptions::default());
        assert_eq!(stats.total_rows, 5);
        assert_eq!(stats.non_empty, 3);
        assert_eq!(stats.empty, 2);
        assert_eq!(stats.distinct, DistinctCount::Exact(2));
        assert_eq!(stats.duplicate_count, Some(1));
    }

    #[test]
    fn numeric_min_max_and_lengths() {
        let doc = doc_from(b"num,pad\n10,x\n2,x\n-5,x\n33,x\n");
        let stats = stats(&doc, 0, &StatsOptions::default());
        assert_eq!(stats.column_type, ColumnType::Integer);
        assert_eq!(stats.min.as_deref(), Some("-5"));
        assert_eq!(stats.max.as_deref(), Some("33"));
        // Several values are two characters; the first one seen wins.
        assert_eq!(stats.longest.as_deref(), Some("10"));
        assert_eq!(stats.shortest.as_deref(), Some("2"));
    }

    /// Data fidelity: the inspector must show the value as it exists in
    /// the file, so padding is preserved. Ordering still ignores padding,
    /// which is why `7` is still the numeric minimum of `" 42 "` and `7`.
    #[test]
    fn padded_values_are_reported_exactly() {
        let doc = doc_from(b"v,pad\n\" 42 \",x\n7,x\n");
        let stats = stats(&doc, 0, &StatsOptions::default());
        assert_eq!(stats.samples.first().map(String::as_str), Some(" 42 "));
        assert_eq!(stats.min.as_deref(), Some("7"));
        assert_eq!(stats.max.as_deref(), Some(" 42 "));
        // Lengths are counted on the exact value, padding included.
        assert_eq!(stats.longest.as_deref(), Some(" 42 "));
        assert_eq!(stats.shortest.as_deref(), Some("7"));
    }

    #[test]
    fn whitespace_only_cells_are_values() {
        // ADR 0003: a whitespace-only cell holds bytes, so it is a value,
        // not empty. The filter `empty` operator already agrees.
        let doc = doc_from(b"v,pad\n\"   \",x\nz,x\n");
        let stats = stats(&doc, 0, &StatsOptions::default());
        assert_eq!(stats.empty, 0);
        assert_eq!(stats.non_empty, 2);
        // Exact-byte min: "   " (0x20) < "z".
        assert_eq!(stats.min.as_deref(), Some("   "));
        assert_eq!(stats.max.as_deref(), Some("z"));
    }

    #[test]
    fn text_min_max_compares_exact_bytes() {
        let doc = doc_from(b"k,pad\nbanana,a\nApple,a\ncherry,a\n");
        let stats = stats(&doc, 0, &StatsOptions::default());
        assert_eq!(stats.min.as_deref(), Some("Apple"));
        assert_eq!(stats.max.as_deref(), Some("cherry"));
    }

    #[test]
    fn text_min_max_is_case_sensitive_and_padding_aware() {
        // Exact bytes: " apple" (0x20) < "Apple" (0x41) < "apple" (0x61).
        let doc = doc_from(b"k,pad\napple,a\nApple,a\n\" apple\",a\n");
        let stats = stats(&doc, 0, &StatsOptions::default());
        assert_eq!(stats.min.as_deref(), Some(" apple"));
        assert_eq!(stats.max.as_deref(), Some("apple"));
    }

    #[test]
    fn frequent_values_are_ranked_by_count_then_value() {
        let doc = doc_from(b"v,pad\nb,a\na,a\nb,a\nc,a\nb,a\na,a\n");
        let stats = stats(&doc, 0, &StatsOptions::default());
        assert_eq!(
            stats.frequent,
            vec![
                ValueCount {
                    value: "b".into(),
                    count: 3,
                },
                ValueCount {
                    value: "a".into(),
                    count: 2,
                },
                ValueCount {
                    value: "c".into(),
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn samples_are_the_first_non_empty_values() {
        let doc = doc_from(b"v,pad\nx,a\n,a\ny,a\nz,a\n");
        let options = StatsOptions {
            samples: 2,
            ..StatsOptions::default()
        };
        let stats = stats(&doc, 0, &options);
        assert_eq!(stats.samples, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn high_cardinality_falls_back_to_approximate() {
        // 10 distinct values with an exact threshold of 3.
        let doc = doc_from(b"v,pad\n0,a\n1,a\n2,a\n3,a\n4,a\n5,a\n6,a\n7,a\n8,a\n9,a\n");
        let options = StatsOptions {
            max_exact_distinct: 3,
            ..StatsOptions::default()
        };
        let stats = stats(&doc, 0, &options);
        match stats.distinct {
            DistinctCount::Approximate(n) => {
                // HLL is approximate; allow a wide band around 10.
                assert!((4..=30).contains(&n), "HLL estimate out of band: {n}");
            }
            other => panic!("expected approximate, got {other:?}"),
        }
        assert!(stats.duplicate_count.is_none());
        assert!(stats.frequent.is_empty());
    }

    #[test]
    fn headerless_column_gets_fallback_name() {
        // A single column has no detected header; `name` is `col0`.
        let doc = doc_from(b"1\n2\n");
        let stats = stats(&doc, 0, &StatsOptions::default());
        assert_eq!(stats.name, "col0");
        assert_eq!(stats.non_empty, 2);
    }

    #[test]
    fn non_numeric_values_do_not_break_numeric_min_max() {
        let doc = doc_from(b"num,pad\n5,x\nn/a,x\n1,x\n9,x\n");
        let stats = stats(&doc, 0, &StatsOptions::default());
        // Inferred as text (only 3 of 4 numeric), so min/max are lexical.
        assert_eq!(stats.min.as_deref(), Some("1"));
        assert_eq!(stats.max.as_deref(), Some("n/a"));
    }

    #[test]
    fn out_of_range_column_is_empty_but_total_is_known() {
        let doc = doc_from(b"a,b\n1,2\n");
        let stats = stats(&doc, 5, &StatsOptions::default());
        assert_eq!(stats.total_rows, 1);
        assert_eq!(stats.non_empty, 0);
        assert_eq!(stats.empty, 0);
        assert_eq!(stats.distinct, DistinctCount::Exact(0));
    }

    #[test]
    fn sample_distinct_dedups_in_first_seen_order_and_skips_empty() {
        let doc = doc_from(b"k,pad\nb,a\na,a\nb,a\n,a\nc,a\n");
        assert_eq!(
            sample_distinct(&doc, 0, 1000, 100),
            vec!["b".to_string(), "a".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn sample_distinct_respects_the_row_budget() {
        let doc = doc_from(b"k,pad\n1,a\n2,a\n3,a\n4,a\n5,a\n");
        // Five rows, a budget of two: the sample is spread (rows 0 and 2).
        assert_eq!(
            sample_distinct(&doc, 0, 2, 100),
            vec!["1".to_string(), "3".to_string()]
        );
    }

    #[test]
    fn sample_distinct_spreads_across_the_whole_file() {
        // 100 rows, all `France` except row 50, which is `Madagascar`.
        // A head-only sample of 10 rows would miss it; the spread sample
        // visits row 50 and includes it.
        let mut csv = String::from("country,pad\n");
        for i in 0..100 {
            let country = if i == 50 { "Madagascar" } else { "France" };
            csv.push_str(&format!("{country},{i}\n"));
        }
        let doc = doc_from(csv.as_bytes());
        let values = sample_distinct(&doc, 0, 10, 100);
        assert!(
            values.contains(&"Madagascar".to_string()),
            "late value missing from sample: {values:?}"
        );
        assert!(values.contains(&"France".to_string()));
    }

    #[test]
    fn sample_distinct_respects_the_distinct_cap_and_stops_early() {
        let doc = doc_from(b"k,pad\n1,a\n2,a\n3,a\n4,a\n");
        assert_eq!(
            sample_distinct(&doc, 0, 100, 2),
            vec!["1".to_string(), "2".to_string()]
        );
    }

    #[test]
    fn sample_distinct_of_an_empty_column_is_empty() {
        let doc = doc_from(b"a,b\n1,\n2,\n");
        assert!(sample_distinct(&doc, 1, 100, 100).is_empty());
    }
}

//! Predicate filtering over a `ParsedDocument`.
//!
//! Filtering is the sibling of [`super::search`]: instead of a free-text
//! needle it evaluates a structured predicate (`column OP value`) over
//! every record and returns the matching record indexes in order.
//!
//! The module is GPUI-free and testable in isolation. Like search it
//! decodes cells lazily from the mmap and never materialises the
//! document; the result is a `Vec<u64>` of record indexes that the UI
//! renders through the same row path as an unfiltered document.

use crate::parser::{ColumnIndex, ParsedDocument};

/// A comparison operator. Text operators are ASCII case-insensitive;
/// the ordering operators compare numerically and skip non-numeric
/// cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    /// `==` — case-insensitive string equality.
    Eq,
    /// `!=` — negation of [`FilterOp::Eq`].
    Ne,
    /// `contains` — case-insensitive substring.
    Contains,
    /// `!contains` / `notcontains` — negation of `Contains`.
    NotContains,
    /// `startswith` / `starts`.
    StartsWith,
    /// `endswith` / `ends`.
    EndsWith,
    /// `empty` — the cell has zero bytes.
    Empty,
    /// `notempty` / `!empty`.
    NonEmpty,
    /// `>` — numeric greater-than.
    Gt,
    /// `>=`.
    Ge,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
}

impl FilterOp {
    /// Parse an operator token. Accepts a few spellings so the user can
    /// type naturally. Returns `None` for anything unrecognised.
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token.to_ascii_lowercase().as_str() {
            "==" | "=" => Self::Eq,
            "!=" | "<>" => Self::Ne,
            "contains" => Self::Contains,
            "!contains" | "notcontains" | "not-contains" => Self::NotContains,
            "startswith" | "starts" | "starts-with" => Self::StartsWith,
            "endswith" | "ends" | "ends-with" => Self::EndsWith,
            "empty" => Self::Empty,
            "notempty" | "!empty" | "not-empty" => Self::NonEmpty,
            ">" => Self::Gt,
            ">=" => Self::Ge,
            "<" => Self::Lt,
            "<=" => Self::Le,
            _ => return None,
        })
    }

    /// Does this operator require a value on the right-hand side?
    pub fn needs_value(self) -> bool {
        !matches!(self, Self::Empty | Self::NonEmpty)
    }

    /// Is this an ordering (numeric) operator?
    pub fn is_numeric(self) -> bool {
        matches!(self, Self::Gt | Self::Ge | Self::Lt | Self::Le)
    }
}

/// A single parsed predicate: `column OP value`. `value` is empty for
/// the value-less operators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub column: usize,
    pub op: FilterOp,
    pub value: String,
}

/// Parse a one-line filter expression.
///
/// Syntax: `<column> <op> [<value>]`. `column` is either a header
/// name (case-insensitive), a 0-based index, or `#N` for an explicit
/// index. `value` may contain spaces. `empty` / `notempty` take no
/// value.
///
/// Examples: `country == US`, `amount > 1000`, `name contains ali`,
/// `email endswith .com`, `notes notempty`, `#3 empty`.
pub fn parse(input: &str, headers: &[String], column_count: usize) -> Result<Filter, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty filter".to_string());
    }
    let mut parts = input.splitn(3, |c: char| c.is_whitespace());
    let column_token = parts.next().unwrap_or("").trim();
    let op_token = parts.next().unwrap_or("").trim();
    let value = parts.next().unwrap_or("").trim().to_string();

    if column_token.is_empty() || op_token.is_empty() {
        return Err("expected `<column> <op> [value]`".to_string());
    }
    let op = FilterOp::parse(op_token).ok_or_else(|| format!("unknown operator `{op_token}`"))?;
    if op.needs_value() && value.is_empty() {
        return Err(format!("operator `{op_token}` needs a value"));
    }
    let column = resolve_column(column_token, headers, column_count)?;
    Ok(Filter { column, op, value })
}

/// Resolve a column token to a 0-based index.
fn resolve_column(token: &str, headers: &[String], column_count: usize) -> Result<usize, String> {
    if let Some(rest) = token.strip_prefix('#') {
        let n: usize = rest
            .parse()
            .map_err(|_| format!("bad column index `{token}`"))?;
        if n >= column_count {
            return Err(format!("column {n} is out of range (0..{column_count})"));
        }
        return Ok(n);
    }
    if let Ok(n) = token.parse::<usize>() {
        if n >= column_count {
            return Err(format!("column {n} is out of range (0..{column_count})"));
        }
        return Ok(n);
    }
    if let Some((index, _)) = headers
        .iter()
        .enumerate()
        .find(|(_, h)| h.eq_ignore_ascii_case(token))
    {
        return Ok(index);
    }
    Err(format!("no column named `{token}`"))
}

/// Evaluate `filters` (combined with AND) over `doc`, returning the
/// matching record indexes in record order. An empty `filters` slice
/// returns no rows; the caller treats "no filter" as a distinct state
/// rather than "match everything".
pub fn apply(doc: &ParsedDocument, filters: &[Filter]) -> Vec<u64> {
    if filters.is_empty() {
        return Vec::new();
    }
    let prepared: Vec<Prepared> = filters.iter().map(Prepared::new).collect();
    let mut out = Vec::new();
    for row in 0..doc.record_count() {
        if prepared.iter().all(|p| p.matches(doc, row)) {
            out.push(row);
        }
    }
    out
}

/// A [`Filter`] with its value pre-lowercased and pre-parsed, so the
/// per-row loop does no repeated work.
struct Prepared {
    column: usize,
    op: FilterOp,
    value_lower: Vec<u8>,
    value_number: Option<f64>,
}

impl Prepared {
    fn new(f: &Filter) -> Self {
        Self {
            column: f.column,
            op: f.op,
            value_lower: f.value.bytes().map(super::ascii_lower).collect(),
            value_number: f.value.trim().parse::<f64>().ok(),
        }
    }

    fn matches(&self, doc: &ParsedDocument, row: u64) -> bool {
        // A missing cell (record shorter than the column) is treated as
        // an empty cell, so `empty` matches it and text predicates do
        // not.
        let cell = doc.cell(row, ColumnIndex(self.column));
        let bytes: &[u8] = match &cell {
            Some(cell) => cell.as_bytes(),
            None => &[],
        };
        match self.op {
            FilterOp::Empty => bytes.is_empty(),
            FilterOp::NonEmpty => !bytes.is_empty(),
            FilterOp::Contains => contains_ci(bytes, &self.value_lower),
            FilterOp::NotContains => !contains_ci(bytes, &self.value_lower),
            FilterOp::StartsWith => starts_with_ci(bytes, &self.value_lower),
            FilterOp::EndsWith => ends_with_ci(bytes, &self.value_lower),
            FilterOp::Eq => eq_ci(bytes, &self.value_lower),
            FilterOp::Ne => !eq_ci(bytes, &self.value_lower),
            FilterOp::Gt | FilterOp::Ge | FilterOp::Lt | FilterOp::Le => {
                let (Some(value), Some(threshold)) = (parse_number(bytes), self.value_number)
                else {
                    return false;
                };
                match self.op {
                    FilterOp::Gt => value > threshold,
                    FilterOp::Ge => value >= threshold,
                    FilterOp::Lt => value < threshold,
                    FilterOp::Le => value <= threshold,
                    _ => false,
                }
            }
        }
    }
}

/// Case-insensitive substring test. `needle_lower` must already be
/// ASCII-lowercased.
fn contains_ci(haystack: &[u8], needle_lower: &[u8]) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    super::ascii_contains_lower(haystack, needle_lower)
}

/// Case-insensitive ASCII equality. `needle_lower` must already be
/// lowercased.
fn eq_ci(haystack: &[u8], needle_lower: &[u8]) -> bool {
    haystack.len() == needle_lower.len()
        && haystack
            .iter()
            .zip(needle_lower)
            .all(|(a, b)| super::ascii_lower(*a) == *b)
}

fn starts_with_ci(haystack: &[u8], prefix_lower: &[u8]) -> bool {
    haystack.len() >= prefix_lower.len()
        && haystack[..prefix_lower.len()]
            .iter()
            .zip(prefix_lower)
            .all(|(a, b)| super::ascii_lower(*a) == *b)
}

fn ends_with_ci(haystack: &[u8], suffix_lower: &[u8]) -> bool {
    haystack.len() >= suffix_lower.len()
        && haystack[haystack.len() - suffix_lower.len()..]
            .iter()
            .zip(suffix_lower)
            .all(|(a, b)| super::ascii_lower(*a) == *b)
}

/// Parse a cell as an `f64`, tolerating surrounding whitespace. Returns
/// `None` for empty or non-numeric cells.
fn parse_number(bytes: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    text.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_opened;
    use crate::parser::ParsedDocument;
    use crate::source::local::open_local;

    fn parse_csv(s: &str) -> ParsedDocument {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("data.csv");
        std::fs::write(&path, s.as_bytes()).expect("write");
        let opened = open_local(&path).expect("open");
        index_opened(opened)
    }

    fn one(input: &str, doc: &ParsedDocument) -> Filter {
        parse(input, &doc.meta.headers, doc.column_count()).expect("parse filter")
    }

    #[test]
    fn parse_resolves_header_by_name_case_insensitively() {
        let doc = parse_csv("Name,City\nalice,Paris\n");
        let f = one("name == alice", &doc);
        assert_eq!(f.column, 0);
        assert_eq!(f.op, FilterOp::Eq);
        assert_eq!(f.value, "alice");

        let f = one("CITY contains par", &doc);
        assert_eq!(f.column, 1);
    }

    #[test]
    fn parse_resolves_numeric_and_hash_columns() {
        let doc = parse_csv("a,b,c\n1,2,3\n");
        assert_eq!(one("1 == x", &doc).column, 1);
        assert_eq!(one("#2 == x", &doc).column, 2);
    }

    #[test]
    fn parse_rejects_unknown_column_and_operator() {
        let doc = parse_csv("a,b\n1,2\n");
        assert!(parse("nope == 1", &doc.meta.headers, doc.column_count()).is_err());
        assert!(parse("a wat 1", &doc.meta.headers, doc.column_count()).is_err());
        assert!(parse("", &doc.meta.headers, doc.column_count()).is_err());
        assert!(parse("a", &doc.meta.headers, doc.column_count()).is_err());
    }

    #[test]
    fn parse_rejects_out_of_range_column() {
        let doc = parse_csv("a,b\n1,2\n");
        assert!(parse("#9 == 1", &doc.meta.headers, doc.column_count()).is_err());
    }

    #[test]
    fn parse_keeps_spaces_in_value() {
        let doc = parse_csv("name,city\nalice,New York\n");
        let f = one("city == New York", &doc);
        assert_eq!(f.value, "New York");
    }

    #[test]
    fn parse_value_less_operators() {
        let doc = parse_csv("a,b\n1,\n");
        let f = one("b empty", &doc);
        assert_eq!(f.op, FilterOp::Empty);
        assert!(f.value.is_empty());
        // `empty` with a trailing value is accepted; the value is
        // ignored by the operator.
        let f = one("a notempty", &doc);
        assert_eq!(f.op, FilterOp::NonEmpty);
    }

    #[test]
    fn eq_is_case_insensitive() {
        let doc = parse_csv("country,x\nUS,1\nus,2\nUk,3\n");
        let matches = apply(&doc, &[one("country == us", &doc)]);
        assert_eq!(matches, vec![0, 1]);
    }

    #[test]
    fn ne_inverts_eq() {
        let doc = parse_csv("country,x\nUS,1\nUK,2\n");
        let matches = apply(&doc, &[one("country != us", &doc)]);
        assert_eq!(matches, vec![1]);
    }

    #[test]
    fn contains_is_case_insensitive_substring() {
        let doc = parse_csv("name,x\nalice,1\nalina,2\nbob,3\n");
        let matches = apply(&doc, &[one("name contains ali", &doc)]);
        assert_eq!(matches, vec![0, 1]);
    }

    #[test]
    fn not_contains_inverts() {
        let doc = parse_csv("name,x\nalice,1\nbob,2\n");
        let matches = apply(&doc, &[one("name !contains ali", &doc)]);
        assert_eq!(matches, vec![1]);
    }

    #[test]
    fn starts_with_and_ends_with() {
        let doc = parse_csv("email,x\na@x.com,1\nb@y.org,2\nc@z.com,3\n");
        assert_eq!(apply(&doc, &[one("email endswith .com", &doc)]), vec![0, 2]);
        assert_eq!(apply(&doc, &[one("email startswith a@", &doc)]), vec![0]);
    }

    #[test]
    fn empty_and_notempty() {
        let doc = parse_csv("a,b\n1,\n,2\n3,4\n");
        assert_eq!(apply(&doc, &[one("b empty", &doc)]), vec![0]);
        assert_eq!(apply(&doc, &[one("b notempty", &doc)]), vec![1, 2]);
    }

    #[test]
    fn numeric_ordering() {
        let doc = parse_csv("amount,x\n10,1\n99,2\n100,3\nabc,4\n-5,5\n");
        assert_eq!(apply(&doc, &[one("amount > 99", &doc)]), vec![2]);
        assert_eq!(apply(&doc, &[one("amount >= 99", &doc)]), vec![1, 2]);
        assert_eq!(apply(&doc, &[one("amount < 10", &doc)]), vec![4]);
        assert_eq!(apply(&doc, &[one("amount <= 10", &doc)]), vec![0, 4]);
    }

    #[test]
    fn numeric_ordering_skips_non_numeric_cells() {
        let doc = parse_csv("amount,x\nabc,1\n");
        assert!(apply(&doc, &[one("amount > 0", &doc)]).is_empty());
    }

    #[test]
    fn missing_cells_count_as_empty() {
        // Row 1 has only one column; an `amount empty` filter should
        // match it.
        let doc = parse_csv("a,amount\n1,5\n2\n");
        assert_eq!(apply(&doc, &[one("amount empty", &doc)]), vec![1]);
    }

    #[test]
    fn filters_combine_with_and() {
        let doc = parse_csv("country,amount\nUS,10\nUS,200\nUK,200\n");
        let filters = vec![one("country == US", &doc), one("amount > 100", &doc)];
        assert_eq!(apply(&doc, &filters), vec![1]);
    }

    #[test]
    fn empty_filter_list_matches_nothing() {
        let doc = parse_csv("a\n1\n2\n");
        assert!(apply(&doc, &[]).is_empty());
    }

    #[test]
    fn quoted_values_are_matched_after_unescaping() {
        let doc = parse_csv("note,x\n\"hello, world\",1\nplain,2\n");
        assert_eq!(
            apply(&doc, &[one("note contains hello, world", &doc)]),
            vec![0]
        );
    }
}

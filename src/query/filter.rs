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
    /// `==` - case-insensitive string equality.
    Eq,
    /// `!=` - negation of [`FilterOp::Eq`].
    Ne,
    /// `contains` - case-insensitive substring.
    Contains,
    /// `!contains` / `notcontains` - negation of `Contains`.
    NotContains,
    /// `startswith` / `starts`.
    StartsWith,
    /// `endswith` / `ends`.
    EndsWith,
    /// `empty` - the cell has zero bytes.
    Empty,
    /// `notempty` / `!empty`.
    NonEmpty,
    /// `>` - numeric greater-than.
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

/// A boolean combination of predicates.
///
/// `AND` binds tighter than `OR`; parentheses override. Built by
/// [`parse_expr`] and evaluated by [`apply_expr`]. The single-predicate
/// [`parse`] / [`apply`] API remains for callers that only need one
/// predicate (or a plain AND of several).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterExpr {
    Predicate(Filter),
    /// `any <op> [value]`: at least one column's cell matches, e.g. `any empty`.
    /// The `Filter.column` field is unused (kept at `0` so the node still
    /// round-trips through `describe` / `parse_expr`).
    AnyColumn(Filter),
    /// `all <op> [value]`: every column's cell matches, e.g. `all notempty`.
    /// Like [`FilterExpr::AnyColumn`], `Filter.column` is an unused `0`.
    AllColumn(Filter),
    And(Box<FilterExpr>, Box<FilterExpr>),
    Or(Box<FilterExpr>, Box<FilterExpr>),
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

/// Resolve a column token to a 0-based index. Shared with
/// `query::suggest`, which needs the same header/index/`#N` rules.
pub(crate) fn resolve_column(
    token: &str,
    headers: &[String],
    column_count: usize,
) -> Result<usize, String> {
    // An exact header match always wins - including names that look like
    // an index (`42`, `col0`) or an explicit `#N`. This is what makes the
    // label the autocomplete offers round-trip back to the column.
    if let Some((index, _)) = headers
        .iter()
        .enumerate()
        .find(|(_, h)| h.eq_ignore_ascii_case(token))
    {
        return Ok(index);
    }
    // Explicit `#N` index (when no header is literally named `#N`).
    if let Some(rest) = token.strip_prefix('#') {
        let n: usize = rest
            .parse()
            .map_err(|_| format!("bad column index `{token}`"))?;
        if n >= column_count {
            return Err(format!("column {n} is out of range (0..{column_count})"));
        }
        return Ok(n);
    }
    // Generated label for a headerless column (`col0`, `col1`, ...).
    if let Some(rest) = token.strip_prefix("col") {
        if let Ok(n) = rest.parse::<usize>() {
            if n < column_count {
                return Ok(n);
            }
        }
    }
    // Normalised fallback: spaces, underscores, and hyphens are treated as
    // equivalent, so a model (or user) writing `last_name` resolves to a
    // column headed `Last Name`. A unique match is required, so an
    // ambiguous token still errors.
    if let Some(index) = normalized_header_match(token, headers) {
        return Ok(index);
    }
    if let Ok(n) = token.parse::<usize>() {
        if n >= column_count {
            return Err(format!("column {n} is out of range (0..{column_count})"));
        }
        return Ok(n);
    }
    Err(format!("no column named `{token}`"))
}

/// Lowercase alphanumeric form of a name, so `Last Name`, `last_name`, and
/// `LastName` all normalise to `lastname`.
fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Unique header whose normalised form equals the token's normalised form.
/// `None` when there is no match or when two headers collide.
fn normalized_header_match(token: &str, headers: &[String]) -> Option<usize> {
    let needle = normalize_name(token);
    if needle.is_empty() {
        return None;
    }
    let mut found = None;
    for (index, header) in headers.iter().enumerate() {
        if normalize_name(header) == needle {
            if found.is_some() {
                return None;
            }
            found = Some(index);
        }
    }
    found
}

/// Parse a filter expression with `AND` / `OR` and parentheses, e.g.
/// `country == US AND (amount > 100 OR status == new)`.
///
/// `AND` binds tighter than `OR`; parentheses override. Keywords are
/// matched case-insensitively as whole unquoted words. Predicate values
/// may contain spaces (`city == New York`); a value that would collide
/// with a keyword can be quoted (`name contains "AND"`). Double quotes
/// around a value are stripped and support a literal `\"`.
///
/// The pseudo-columns `any` and `all` scope a predicate across every
/// column, so `any empty` matches a row with at least one blank cell.
/// A real column literally named `any` or `all` is unreachable by name;
/// use its `#N` index instead.
pub fn parse_expr(
    input: &str,
    headers: &[String],
    column_count: usize,
) -> Result<FilterExpr, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty filter".to_string());
    }
    let tokens = tokenize(input)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        headers,
        column_count,
    };
    let expr = parser.parse_expr()?;
    if let Some(token) = parser.peek() {
        return Err(match token {
            Token::LParen => "unexpected '('".to_string(),
            Token::RParen => "unexpected ')'".to_string(),
            Token::Word { text, .. } => format!("unexpected `{text}`"),
        });
    }
    Ok(expr)
}

/// One lexical token for [`parse_expr`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    LParen,
    RParen,
    /// A word; `quoted` marks a value that must never be read as a
    /// boolean keyword.
    Word {
        text: String,
        quoted: bool,
    },
}

/// True for an unquoted `AND` (`Some(true)`) or `OR` (`Some(false)`).
fn keyword_of(token: &Token) -> Option<bool> {
    match token {
        Token::Word {
            text,
            quoted: false,
        } if text.eq_ignore_ascii_case("and") => Some(true),
        Token::Word {
            text,
            quoted: false,
        } if text.eq_ignore_ascii_case("or") => Some(false),
        _ => None,
    }
}

/// Split `input` into tokens. `(` and `)` are standalone even when
/// adjacent to a word. A double-quoted run becomes one word with the
/// quotes removed and `\"` unescaped; an unterminated quote is an error.
fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b if b.is_ascii_whitespace() => i += 1,
            b'(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            b'"' => {
                i += 1;
                let mut text = String::new();
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'"') {
                        text.push('"');
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        closed = true;
                        i += 1;
                        break;
                    }
                    // Advance a whole UTF-8 char so `i` stays on a
                    // char boundary for the slice below.
                    let ch = input[i..].chars().next().expect("non-empty remainder");
                    text.push(ch);
                    i += ch.len_utf8();
                }
                if !closed {
                    return Err("unterminated quoted value".to_string());
                }
                tokens.push(Token::Word { text, quoted: true });
            }
            _ => {
                let start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && bytes[i] != b'('
                    && bytes[i] != b')'
                {
                    i += 1;
                }
                tokens.push(Token::Word {
                    text: input[start..i].to_string(),
                    quoted: false,
                });
            }
        }
    }
    Ok(tokens)
}

/// Recursive-descent parser over the token stream.
///
/// ```text
/// expr   := term (OR term)*
/// term   := factor (AND factor)*
/// factor := '(' expr ')' | predicate
/// predicate := WORD OP VALUE?
/// ```
struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    headers: &'a [String],
    column_count: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Consume the next word, if the next token is one.
    fn next_word(&mut self) -> Option<String> {
        match self.tokens.get(self.pos).cloned() {
            Some(Token::Word { text, .. }) => {
                self.pos += 1;
                Some(text)
            }
            _ => None,
        }
    }

    fn parse_expr(&mut self) -> Result<FilterExpr, String> {
        let mut left = self.parse_term()?;
        while let Some(token) = self.peek().cloned() {
            if keyword_of(&token) != Some(false) {
                break;
            }
            self.pos += 1;
            let right = self.parse_term()?;
            left = FilterExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<FilterExpr, String> {
        let mut left = self.parse_factor()?;
        while let Some(token) = self.peek().cloned() {
            if keyword_of(&token) != Some(true) {
                break;
            }
            self.pos += 1;
            let right = self.parse_factor()?;
            left = FilterExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<FilterExpr, String> {
        match self.peek().cloned() {
            Some(Token::LParen) => {
                self.pos += 1;
                let inner = self.parse_expr()?;
                if matches!(self.peek(), Some(Token::RParen)) {
                    self.pos += 1;
                    Ok(inner)
                } else {
                    Err("expected ')'".to_string())
                }
            }
            Some(Token::RParen) => Err("unexpected ')'".to_string()),
            Some(Token::Word { .. }) => self.parse_predicate(),
            None => Err("expected a predicate".to_string()),
        }
    }

    fn parse_predicate(&mut self) -> Result<FilterExpr, String> {
        let Some(column_token) = self.next_word() else {
            return Err("expected a column name".to_string());
        };
        let Some(op_token) = self.next_word() else {
            return Err(format!("expected an operator after `{column_token}`"));
        };
        let op =
            FilterOp::parse(&op_token).ok_or_else(|| format!("unknown operator `{op_token}`"))?;
        let value = if op.needs_value() {
            let mut parts: Vec<String> = Vec::new();
            // A value runs until the next boolean keyword, paren, or end
            // of input, so `city == New York AND ...` keeps the space.
            while let Some(token) = self.tokens.get(self.pos).cloned() {
                match token {
                    Token::Word { text, .. } if keyword_of(&token).is_none() => {
                        parts.push(text);
                        self.pos += 1;
                    }
                    _ => break,
                }
            }
            if parts.is_empty() {
                return Err(format!("operator `{op_token}` needs a value"));
            }
            parts.join(" ")
        } else {
            String::new()
        };
        // `any` / `all` scope the predicate across every column instead of
        // resolving a single column index.
        match column_token.to_ascii_lowercase().as_str() {
            "any" => Ok(FilterExpr::AnyColumn(Filter {
                column: 0,
                op,
                value,
            })),
            "all" => Ok(FilterExpr::AllColumn(Filter {
                column: 0,
                op,
                value,
            })),
            _ => {
                let column = resolve_column(&column_token, self.headers, self.column_count)?;
                Ok(FilterExpr::Predicate(Filter { column, op, value }))
            }
        }
    }
}

/// Evaluate `filters` (combined with AND) over `doc`, returning the
/// matching record indexes in record order. An empty `filters` slice
/// returns no rows; the caller treats "no filter" as a distinct state
/// rather than "match everything".
pub fn apply(doc: &ParsedDocument, filters: &[Filter]) -> Vec<u64> {
    let Some(expr) = and_all(filters) else {
        return Vec::new();
    };
    apply_expr(doc, &expr)
}

/// Fold a plain slice of predicates into a left-associated AND
/// expression. Returns `None` for an empty slice.
fn and_all(filters: &[Filter]) -> Option<FilterExpr> {
    let mut iter = filters.iter();
    let first = iter.next()?;
    let mut expr = FilterExpr::Predicate(first.clone());
    for filter in iter {
        expr = FilterExpr::And(
            Box::new(expr),
            Box::new(FilterExpr::Predicate(filter.clone())),
        );
    }
    Some(expr)
}

/// Evaluate an arbitrary boolean expression over `doc`, returning the
/// matching record indexes in record order.
pub fn apply_expr(doc: &ParsedDocument, expr: &FilterExpr) -> Vec<u64> {
    let prepared = PreparedExpr::new(expr);
    let mut out = Vec::new();
    for row in 0..doc.record_count() {
        if prepared.matches(doc, row) {
            out.push(row);
        }
    }
    out
}

/// A [`FilterExpr`] with every predicate's value pre-lowercased and
/// pre-parsed, so the per-row loop does no repeated work. Built once
/// per apply; `matches` short-circuits on `AND` / `OR`.
enum PreparedExpr {
    Predicate(Prepared),
    AnyColumn(Prepared),
    AllColumn(Prepared),
    And(Box<PreparedExpr>, Box<PreparedExpr>),
    Or(Box<PreparedExpr>, Box<PreparedExpr>),
}

impl PreparedExpr {
    fn new(expr: &FilterExpr) -> Self {
        match expr {
            FilterExpr::Predicate(filter) => PreparedExpr::Predicate(Prepared::new(filter)),
            FilterExpr::AnyColumn(filter) => PreparedExpr::AnyColumn(Prepared::new(filter)),
            FilterExpr::AllColumn(filter) => PreparedExpr::AllColumn(Prepared::new(filter)),
            FilterExpr::And(a, b) => {
                PreparedExpr::And(Box::new(Self::new(a)), Box::new(Self::new(b)))
            }
            FilterExpr::Or(a, b) => {
                PreparedExpr::Or(Box::new(Self::new(a)), Box::new(Self::new(b)))
            }
        }
    }

    fn matches(&self, doc: &ParsedDocument, row: u64) -> bool {
        match self {
            PreparedExpr::Predicate(prepared) => prepared.matches(doc, row),
            PreparedExpr::AnyColumn(prepared) => {
                (0..doc.column_count()).any(|column| prepared.matches_column(doc, row, column))
            }
            // A document with no columns cannot have "all" of them match.
            PreparedExpr::AllColumn(prepared) => {
                doc.column_count() > 0
                    && (0..doc.column_count())
                        .all(|column| prepared.matches_column(doc, row, column))
            }
            PreparedExpr::And(a, b) => a.matches(doc, row) && b.matches(doc, row),
            PreparedExpr::Or(a, b) => a.matches(doc, row) || b.matches(doc, row),
        }
    }
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
        self.matches_column(doc, row, self.column)
    }

    /// Fetch one cell and test it against this predicate. Shared by the
    /// per-column and `any` / `all` row-scope paths. A missing cell
    /// (record shorter than the column) is treated as an empty cell, so
    /// `empty` matches it and text predicates do not.
    fn matches_column(&self, doc: &ParsedDocument, row: u64, column: usize) -> bool {
        let cell = doc.cell(row, ColumnIndex(column));
        let bytes: &[u8] = match &cell {
            Some(cell) => cell.as_bytes(),
            None => &[],
        };
        self.matches_cell(bytes)
    }

    /// Test one cell's bytes against this predicate.
    fn matches_cell(&self, bytes: &[u8]) -> bool {
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
    fn resolve_column_normalises_spaced_headers() {
        let headers = vec!["Last Name".to_string(), "pad".to_string()];
        assert_eq!(resolve_column("last_name", &headers, 2).unwrap(), 0);
        assert_eq!(resolve_column("lastname", &headers, 2).unwrap(), 0);
        assert_eq!(resolve_column("Last Name", &headers, 2).unwrap(), 0);
        // The real parser accepts the normalised form too.
        assert_eq!(
            parse_expr("last_name startswith C", &headers, 2).unwrap(),
            FilterExpr::Predicate(Filter {
                column: 0,
                op: FilterOp::StartsWith,
                value: "C".into(),
            })
        );
        // Two headers that normalise the same are ambiguous, not guessed.
        let dup = vec!["Last Name".to_string(), "last_name".to_string()];
        assert!(resolve_column("lastname", &dup, 2).is_err());
    }

    #[test]
    fn resolve_column_round_trips_generated_and_numeric_labels() {
        // Headerless columns are offered (and typed) as `colN`.
        assert_eq!(resolve_column("col0", &[], 3).unwrap(), 0);
        assert_eq!(resolve_column("col2", &[], 3).unwrap(), 2);
        assert!(resolve_column("col9", &[], 3).is_err());
        // A column literally named `42` resolves to itself, not index 42.
        let headers = vec!["42".to_string(), "name".to_string()];
        assert_eq!(resolve_column("42", &headers, 2).unwrap(), 0);
        // A plain index still works when no header matches it.
        assert_eq!(resolve_column("1", &headers, 2).unwrap(), 1);
        // `#N` is always an index when no header matches it.
        assert_eq!(resolve_column("#1", &headers, 2).unwrap(), 1);
        assert!(resolve_column("#foo", &headers, 2).is_err());
        // A column literally named `#2` resolves to the header (the
        // autocomplete can offer it and it must round-trip).
        let hash_headers = vec!["id".to_string(), "#2".to_string(), "name".to_string()];
        assert_eq!(resolve_column("#2", &hash_headers, 3).unwrap(), 1);
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

    // ---- compound predicates (AND / OR / parentheses) ----------------

    fn expr(input: &str, doc: &ParsedDocument) -> FilterExpr {
        parse_expr(input, &doc.meta.headers, doc.column_count()).expect("parse expression")
    }

    #[test]
    fn any_empty_matches_a_row_with_a_blank_cell() {
        let doc = parse_csv("a,b,c\n1,,3\n1,2,3\n,2,3\n");
        let parsed = expr("any empty", &doc);
        assert_eq!(
            parsed,
            FilterExpr::AnyColumn(Filter {
                column: 0,
                op: FilterOp::Empty,
                value: String::new(),
            })
        );
        assert_eq!(apply_expr(&doc, &parsed), vec![0, 2]);
    }

    #[test]
    fn all_notempty_requires_every_column() {
        // Row 2 is short, so its missing cells count as empty.
        let doc = parse_csv("a,b,c\n1,,3\n1,2,3\n1\n");
        assert_eq!(apply_expr(&doc, &expr("all notempty", &doc)), vec![1]);
    }

    #[test]
    fn whitespace_only_cells_are_not_empty() {
        // ADR 0003: `empty` means zero bytes, so a whitespace-only cell
        // is a value and neither matches `empty` nor fails `notempty`.
        let doc = parse_csv("v,pad\n\"   \",x\n,y\n");
        assert_eq!(apply_expr(&doc, &expr("v empty", &doc)), vec![1]);
        assert_eq!(apply_expr(&doc, &expr("v notempty", &doc)), vec![0]);
    }

    #[test]
    fn row_scope_any_contains_is_case_insensitive() {
        let doc = parse_csv("a,b\nfoo,\n,zap\n");
        assert_eq!(apply_expr(&doc, &expr("any contains ZAP", &doc)), vec![1]);
    }

    #[test]
    fn row_scope_any_ordering_skips_non_numeric_cells() {
        let doc = parse_csv("a,b\nx,1\nx,9\n");
        assert_eq!(apply_expr(&doc, &expr("any > 5", &doc)), vec![1]);
    }

    #[test]
    fn row_scope_parses_inside_boolean_expressions() {
        let doc = parse_csv("a,b\n1,2\n3,4\n5,6\n");
        assert_eq!(
            apply_expr(&doc, &expr("(any > 4) AND a == 5", &doc)),
            vec![2]
        );
    }

    #[test]
    fn row_scope_rejects_missing_value_and_unknown_operator() {
        let doc = parse_csv("a,b\n1,2\n");
        let headers = &doc.meta.headers;
        let count = doc.column_count();
        assert!(parse_expr("any", headers, count).is_err());
        assert!(parse_expr("any ==", headers, count).is_err());
        assert!(parse_expr("all badop 1", headers, count).is_err());
    }

    #[test]
    fn expr_and_binds_tighter_than_or() {
        // a == 1 OR a == 2 AND b == 3  ==  a == 1 OR (a == 2 AND b == 3)
        let doc = parse_csv("a,b\n1,9\n1,3\n2,3\n2,9\n");
        let parsed = expr("a == 1 OR a == 2 AND b == 3", &doc);
        assert_eq!(
            parsed,
            FilterExpr::Or(
                Box::new(FilterExpr::Predicate(Filter {
                    column: 0,
                    op: FilterOp::Eq,
                    value: "1".into(),
                })),
                Box::new(FilterExpr::And(
                    Box::new(FilterExpr::Predicate(Filter {
                        column: 0,
                        op: FilterOp::Eq,
                        value: "2".into(),
                    })),
                    Box::new(FilterExpr::Predicate(Filter {
                        column: 1,
                        op: FilterOp::Eq,
                        value: "3".into(),
                    })),
                )),
            )
        );
        assert_eq!(apply_expr(&doc, &parsed), vec![0, 1, 2]);
    }

    #[test]
    fn expr_parentheses_override_precedence() {
        let doc = parse_csv("a,b\n1,9\n1,3\n2,3\n2,9\n");
        let parsed = expr("(a == 1 OR a == 2) AND b == 3", &doc);
        assert_eq!(apply_expr(&doc, &parsed), vec![1, 2]);
        // Redundant nesting still parses to the inner predicate.
        assert_eq!(
            expr("((a == 1))", &doc),
            FilterExpr::Predicate(Filter {
                column: 0,
                op: FilterOp::Eq,
                value: "1".into(),
            })
        );
    }

    #[test]
    fn expr_keeps_multi_word_value_before_boolean_operator() {
        let doc = parse_csv("city,amount\nNew York,50\nNew York,200\nBoston,200\n");
        let parsed = expr("city == New York AND amount > 100", &doc);
        let FilterExpr::And(left, _) = &parsed else {
            panic!("expected an AND at the root, got {parsed:?}");
        };
        assert_eq!(
            **left,
            FilterExpr::Predicate(Filter {
                column: 0,
                op: FilterOp::Eq,
                value: "New York".into(),
            })
        );
        assert_eq!(apply_expr(&doc, &parsed), vec![1]);
    }

    #[test]
    fn expr_quoted_values_can_contain_keywords_and_spaces() {
        let doc = parse_csv("name,x\nAND,1\nBob,2\nNew York,3\n");
        assert_eq!(
            apply_expr(&doc, &expr("name contains \"AND\"", &doc)),
            vec![0]
        );
        assert_eq!(
            apply_expr(&doc, &expr("name == \"New York\"", &doc)),
            vec![2]
        );
        // A quoted `OR` is a literal, not an operator.
        assert_eq!(
            apply_expr(&doc, &expr("name == \"or\"", &doc)),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn expr_keywords_are_case_insensitive() {
        let doc = parse_csv("a,b\n1,2\n3,4\n");
        assert_eq!(apply_expr(&doc, &expr("a == 1 and b == 2", &doc)), vec![0]);
        assert_eq!(
            apply_expr(&doc, &expr("a == 1 oR a == 3", &doc)),
            vec![0, 1]
        );
    }

    #[test]
    fn expr_parse_errors() {
        let doc = parse_csv("a,b\n1,2\n");
        let headers = &doc.meta.headers;
        let n = doc.column_count();
        assert!(parse_expr("", headers, n).is_err());
        assert!(parse_expr("(a == 1", headers, n).is_err());
        assert!(parse_expr("a == 1)", headers, n).is_err());
        assert!(parse_expr("a == 1 AND", headers, n).is_err());
        assert!(parse_expr("a == AND b == 2", headers, n).is_err());
        assert!(parse_expr("a notempty extra", headers, n).is_err());
        assert!(parse_expr("a == 1 OR )", headers, n).is_err());
    }

    #[test]
    fn numeric_ordering_accepts_decimal_thresholds() {
        let doc = parse_csv("amount,x\n999,a\n1000,a\n1000.5,a\n");
        assert_eq!(apply(&doc, &[one("amount > 1000.0", &doc)]), vec![2]);
        assert_eq!(apply(&doc, &[one("amount >= 1000.0", &doc)]), vec![1, 2]);
        assert_eq!(apply(&doc, &[one("amount < 1000.0", &doc)]), vec![0]);
    }

    #[test]
    fn expr_single_predicate_matches_parse_and_apply() {
        let doc = parse_csv("country,x\nUS,1\nUK,2\n");
        let single = parse("country == us", &doc.meta.headers, doc.column_count()).unwrap();
        let parsed = parse_expr("country == us", &doc.meta.headers, doc.column_count()).unwrap();
        assert_eq!(parsed, FilterExpr::Predicate(single.clone()));
        assert_eq!(apply_expr(&doc, &parsed), apply(&doc, &[single]));
    }

    #[test]
    fn expr_or_returns_each_matching_row_once_and_short_circuits() {
        let doc = parse_csv("a,b\n1,0\n2,0\n3,0\n");
        // A row matching both sides of an OR appears once.
        assert_eq!(apply_expr(&doc, &expr("a == 1 OR a == 1", &doc)), vec![0]);
        // OR union across disjoint operands.
        assert_eq!(
            apply_expr(&doc, &expr("a == 1 OR a == 3", &doc)),
            vec![0, 2]
        );
    }
}

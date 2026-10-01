//! Rules-based natural-language filter parsing (Milestone 6, Tier 1).
//!
//! Turns a phrase such as `country is US and amount over 100` into the
//! same [`FilterExpr`] the filter bar builds, using only the schema
//! context (column names, types, and sampled distinct values). It is
//! deliberately deterministic and dependency-free: it never guesses
//! silently, and it reports the tokens it could not consume so the caller
//! can fall back to an optional model or let the user edit.
//!
//! See `docs/adr/0001-natural-language-ask.md` for the escalation
//! contract. This module is GPUI-free and unit-testable.

use std::collections::HashMap;

use crate::schema::ColumnType;

use super::filter::{Filter, FilterExpr, FilterOp};

/// One column's schema context for the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NlColumn {
    pub name: String,
    pub column_type: ColumnType,
}

/// The context the parser is allowed to use. Values are fetched lazily
/// per column through the `values` callback so the parser never scans a
/// column it does not reference.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NlSchema {
    pub columns: Vec<NlColumn>,
    /// Extra lowercased alias to column-index mappings, e.g.
    /// `customer` -> `customer_id`.
    pub aliases: Vec<(String, usize)>,
}

/// The outcome of a parse.
#[derive(Debug, Clone, PartialEq)]
pub struct NlResult {
    /// The parsed expression, or `None` when no predicate was produced.
    pub expr: Option<FilterExpr>,
    /// Tokens/phrases the parser could not consume.
    pub unmatched: Vec<String>,
    /// True when the phrase was fully and unambiguously consumed. The
    /// caller only uses the result without escalation when this is true.
    pub clean: bool,
    /// True when the expression came from the Tier 2 model rather than
    /// the rules.
    pub from_model: bool,
}

/// A natural-language front end. Tier 1 is [`RuleBackend`]; an optional
/// local model would implement the same trait (see the ADR).
pub trait NlBackend {
    fn parse(&self, phrase: &str, schema: &NlSchema) -> NlResult;
}

/// The deterministic Tier 1 backend. `values(column)` returns the sampled
/// distinct values for that column (or an empty slice when unknown).
pub struct RuleBackend<F> {
    values: F,
}

impl<F> RuleBackend<F>
where
    F: Fn(usize) -> Vec<String>,
{
    pub fn new(values: F) -> Self {
        Self { values }
    }
}

impl<F> NlBackend for RuleBackend<F>
where
    F: Fn(usize) -> Vec<String>,
{
    fn parse(&self, phrase: &str, schema: &NlSchema) -> NlResult {
        parse(phrase, schema, &self.values)
    }
}

/// Parse `phrase` against `schema`, binding values through `values`.
pub fn parse<F>(phrase: &str, schema: &NlSchema, values: &F) -> NlResult
where
    F: Fn(usize) -> Vec<String>,
{
    let tokens = tokenize(phrase);
    if tokens.is_empty() {
        return NlResult {
            expr: None,
            unmatched: Vec::new(),
            clean: false,
            from_model: false,
        };
    }
    // Row-scope phrasing (`at least one column is blank`, `all columns
    // filled`) is recognized before the per-column loop. It never uses the
    // per-column value samples, and falls through when it does not match.
    if let Some(result) = parse_row_scope(&tokens) {
        return result;
    }
    let lookup = column_lookup(schema);
    let phrases = operator_phrases();

    let mut expr: Option<FilterExpr> = None;
    let mut pending_and = true;
    let mut unmatched = Vec::new();
    let mut clean = true;
    // The last (column, op) pair, so `status is failed or refunded` can
    // reuse the column for the second value.
    let mut last: Option<(usize, FilterOp)> = None;
    let mut pos = 0;
    // True when the next token should be a column, so filler words can be
    // skipped without eating a value.
    let mut expect_column = true;

    while pos < tokens.len() {
        if let Some(and) = conjunction_of(&tokens[pos].lower) {
            pending_and = and;
            pos += 1;
            expect_column = true;
            continue;
        }
        if tokens[pos].lower == "either" {
            pos += 1;
            continue;
        }
        if expect_column {
            // Skip leading filler ("show me", "list all", ...).
            while pos < tokens.len() && is_filler(&tokens[pos].lower) {
                pos += 1;
            }
            if pos >= tokens.len() {
                break;
            }
        }

        // A bare value continuing the previous predicate (`... or refunded`).
        if let Some((column, op)) = last {
            if let Some((value, next)) = value_only(&tokens, pos, column, values) {
                let predicate = FilterExpr::Predicate(Filter { column, op, value });
                expr = Some(combine(expr.take(), predicate, pending_and));
                pending_and = true;
                pos = next;
                continue;
            }
        }

        let Some((column, consumed)) =
            match_column(&tokens, pos, &lookup).or_else(|| fuzzy_column(&tokens, pos, &lookup))
        else {
            unmatched.push(tokens[pos].text.clone());
            clean = false;
            pos += 1;
            continue;
        };
        pos += consumed;
        expect_column = false;

        // `between A and B` is a range on one column.
        if tokens.get(pos).map(|t| t.lower.as_str()) == Some("between") {
            if let Some((range, next, bound)) = parse_between(&tokens, pos, column, values) {
                clean = clean && bound;
                pos = next;
                expr = Some(combine(expr.take(), range, pending_and));
                pending_and = true;
                last = Some((column, FilterOp::Ge));
                continue;
            }
        }

        let Some((op, op_consumed)) = match_operator(&tokens, pos, &phrases) else {
            let rest = tokens[pos..]
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            unmatched.push(rest);
            clean = false;
            break;
        };
        pos += op_consumed;

        let (value, next, bound) = bind_value(&tokens, pos, op, column, values);
        if op.needs_value() && !bound {
            clean = false;
            if !value.is_empty() {
                unmatched.push(value.clone());
            }
            // No progress means the same token would be retried forever.
            if next == pos {
                break;
            }
        }
        pos = next;
        let predicate = FilterExpr::Predicate(Filter { column, op, value });
        expr = Some(combine(expr.take(), predicate, pending_and));
        pending_and = true;
        last = Some((column, op));
    }

    if expr.is_none() {
        clean = false;
    }
    NlResult {
        expr,
        unmatched,
        clean,
        from_model: false,
    }
}

/// Combine a previous expression with a new predicate using AND or OR.
fn combine(prev: Option<FilterExpr>, predicate: FilterExpr, and: bool) -> FilterExpr {
    match prev {
        None => predicate,
        Some(prev) if and => FilterExpr::And(Box::new(prev), Box::new(predicate)),
        Some(prev) => FilterExpr::Or(Box::new(prev), Box::new(predicate)),
    }
}

/// Match a whole row-scope phrase and return a clean [`NlResult`] with an
/// [`FilterExpr::AnyColumn`] / [`FilterExpr::AllColumn`] expression.
/// Handles both `any column is empty` (column-first) and
/// `any row that has an empty column` (row-first). Returns `None` so the
/// caller falls through to the per-column parser when nothing matches.
fn parse_row_scope(tokens: &[Token]) -> Option<NlResult> {
    parse_column_scope(tokens).or_else(|| parse_row_noun_scope(tokens))
}

/// `any column is empty`, `at least one column is blank`, `all columns
/// filled`, `no column is empty`.
fn parse_column_scope(tokens: &[Token]) -> Option<NlResult> {
    let start = skip_scope_filler(tokens, 0);
    let (any, scope_negate, next) = row_scope_marker(tokens, start)?;
    let (op, value) = row_scope_operator(tokens, next)?;
    let op = if scope_negate {
        row_scope_negate(op)?
    } else {
        op
    };
    Some(row_scope_result(!any, op, value))
}

/// `<row noun> ... has/with <negation|article> <empty-word> <column-word>`,
/// e.g. `any row that has an empty column` or `rows with no blank cells`.
/// A negated row-has phrase is universal over columns (`has no empty
/// column` means every column is non-empty), so it maps to `AllColumn`.
fn parse_row_noun_scope(tokens: &[Token]) -> Option<NlResult> {
    let at = |i: usize| tokens.get(i).map(|t| t.lower.as_str());
    let mut p = skip_row_scope_filler(tokens, 0);
    // `no row ...` negates the whole row selection; `any`/`every` on the
    // row quantifier does not change the column scope.
    let mut negate = false;
    if at(p) == Some("no") {
        negate = true;
        p += 1;
    } else if matches!(at(p), Some("any" | "some" | "all" | "every")) {
        p += 1;
    }
    if !is_row_word(at(p)?) {
        return None;
    }
    p += 1;
    if matches!(at(p), Some("that" | "which" | "where" | "who")) {
        p += 1;
    }
    if !matches!(
        at(p),
        Some(
            "has"
                | "have"
                | "contain"
                | "contains"
                | "containing"
                | "with"
                | "include"
                | "includes"
                | "including"
        )
    ) {
        return None;
    }
    p += 1;
    if matches!(at(p), Some("no") | Some("not")) {
        negate = true;
        p += 1;
    } else if at(p) == Some("at") && at(p + 1) == Some("least") {
        p += 2;
        if matches!(at(p), Some("one") | Some("1")) {
            p += 1;
        }
        if matches!(at(p), Some("a") | Some("an") | Some("any")) {
            p += 1;
        }
    } else if matches!(at(p), Some("a") | Some("an") | Some("any") | Some("some")) {
        p += 1;
    }
    // Require `<empty-word> <column-word>` to end the phrase, so ordinary
    // `rows with amount over 100` falls through to the column parser.
    let op = adjective_op(at(p)?)?;
    if !is_column_word(at(p + 1)?) || p + 2 != tokens.len() {
        return None;
    }
    let (all, op) = if negate {
        (true, row_scope_negate(op)?)
    } else {
        (false, op)
    };
    Some(row_scope_result(all, op, String::new()))
}

fn row_scope_result(all: bool, op: FilterOp, value: String) -> NlResult {
    let filter = Filter {
        column: 0,
        op,
        value,
    };
    let expr = if all {
        FilterExpr::AllColumn(filter)
    } else {
        FilterExpr::AnyColumn(filter)
    };
    NlResult {
        expr: Some(expr),
        unmatched: Vec::new(),
        clean: true,
        from_model: false,
    }
}

fn adjective_op(word: &str) -> Option<FilterOp> {
    Some(match word {
        "empty" | "blank" | "missing" | "null" => FilterOp::Empty,
        "filled" | "present" | "populated" | "nonempty" | "non-empty" => FilterOp::NonEmpty,
        _ => return None,
    })
}

fn is_row_word(word: &str) -> bool {
    matches!(
        word,
        "row" | "rows" | "record" | "records" | "entry" | "entries" | "line" | "lines"
    )
}

/// Leading words that do not change a row-scope phrase. Unlike
/// [`is_filler`], the scope markers themselves (`any`, `all`, `no`, `at`)
/// are never skipped.
/// Leading filler for the column-first matcher; row nouns are skipped as
/// filler here (`showing entries where any column is blank`).
fn skip_scope_filler(tokens: &[Token], mut pos: usize) -> usize {
    while pos < tokens.len() {
        let word = tokens[pos].lower.as_str();
        if matches!(word, "any" | "some" | "a" | "all" | "every" | "no" | "at") {
            break;
        }
        if is_row_filler(word) {
            pos += 1;
        } else {
            break;
        }
    }
    pos
}

/// Leading filler for the row-first matcher; a row noun is the anchor and
/// stops the skip (`show rows with an empty cell`).
fn skip_row_scope_filler(tokens: &[Token], mut pos: usize) -> usize {
    while pos < tokens.len() {
        let word = tokens[pos].lower.as_str();
        if matches!(word, "any" | "some" | "a" | "all" | "every" | "no" | "at") {
            break;
        }
        if is_row_word(word) {
            break;
        }
        if is_row_filler(word) {
            pos += 1;
        } else {
            break;
        }
    }
    pos
}

fn is_row_filler(word: &str) -> bool {
    is_filler(word)
        || matches!(
            word,
            "showing" | "listing" | "finding" | "displaying" | "getting" | "seeing"
        )
}

/// Match the scope prefix, returning `(is_any, negate, next_token)`.
/// `negate` is true for `no column ...`, which flips the inner operator.
fn row_scope_marker(tokens: &[Token], pos: usize) -> Option<(bool, bool, usize)> {
    let at = |i: usize| tokens.get(i).map(|t| t.lower.as_str());
    let mut p = pos;
    match at(p)? {
        "at" => {
            if at(p + 1) != Some("least") {
                return None;
            }
            p += 2;
            if matches!(at(p), Some("one") | Some("1")) {
                p += 1;
            }
            let p = skip_of_the(tokens, p);
            if !is_column_word(at(p)?) {
                return None;
            }
            Some((true, false, p + 1))
        }
        scope => {
            let (any, negate) = match scope {
                "any" | "some" | "a" => (true, false),
                "all" | "every" => (false, false),
                "no" => (false, true),
                _ => return None,
            };
            p += 1;
            let p = skip_of_the(tokens, p);
            if !is_column_word(at(p)?) {
                return None;
            }
            Some((any, negate, p + 1))
        }
    }
}

fn skip_of_the(tokens: &[Token], mut pos: usize) -> usize {
    while matches!(
        tokens.get(pos).map(|t| t.lower.as_str()),
        Some("of") | Some("the")
    ) {
        pos += 1;
    }
    pos
}

fn is_column_word(word: &str) -> bool {
    matches!(
        word,
        "column" | "columns" | "field" | "fields" | "cell" | "cells" | "value" | "values"
    )
}

/// Match the comparison after a row-scope prefix. The match must consume
/// the whole remainder of the phrase, so trailing junk falls through.
fn row_scope_operator(tokens: &[Token], pos: usize) -> Option<(FilterOp, String)> {
    let word = |i: usize| tokens.get(i).map(|t| t.lower.as_str());
    let end = tokens.len();
    let mut p = pos;
    if matches!(
        word(p),
        Some("is" | "are" | "was" | "were" | "does" | "do" | "did")
    ) {
        p += 1;
    }
    let negate = word(p) == Some("not");
    if negate {
        p += 1;
    }
    let (op, value, endpos) = match word(p)? {
        "empty" | "blank" | "missing" | "null" => (FilterOp::Empty, String::new(), p + 1),
        "filled" | "present" | "populated" | "nonempty" | "non-empty" => {
            (FilterOp::NonEmpty, String::new(), p + 1)
        }
        "has" | "have" => {
            let q = if matches!(word(p + 1), Some("a") | Some("an")) {
                p + 2
            } else {
                p + 1
            };
            if matches!(word(q), Some("value") | Some("values")) {
                (FilterOp::NonEmpty, String::new(), q + 1)
            } else {
                return None;
            }
        }
        "contains" | "contain" | "includes" | "include" | "including" | "containing" => {
            (FilterOp::Contains, join_tokens(tokens, p + 1, end), end)
        }
        "starts" if word(p + 1) == Some("with") => {
            (FilterOp::StartsWith, join_tokens(tokens, p + 2, end), end)
        }
        "ends" if word(p + 1) == Some("with") => {
            (FilterOp::EndsWith, join_tokens(tokens, p + 2, end), end)
        }
        "equals" | "equal" => {
            let q = if word(p + 1) == Some("to") {
                p + 2
            } else {
                p + 1
            };
            (FilterOp::Eq, join_tokens(tokens, q, end), end)
        }
        "over" | "above" | "exceeds" => (FilterOp::Gt, join_tokens(tokens, p + 1, end), end),
        "under" | "below" => (FilterOp::Lt, join_tokens(tokens, p + 1, end), end),
        "greater" | "more" if word(p + 1) == Some("than") => {
            (FilterOp::Gt, join_tokens(tokens, p + 2, end), end)
        }
        "less" | "fewer" if word(p + 1) == Some("than") => {
            (FilterOp::Lt, join_tokens(tokens, p + 2, end), end)
        }
        "at" if word(p + 1) == Some("least") => {
            (FilterOp::Ge, join_tokens(tokens, p + 2, end), end)
        }
        "at" if word(p + 1) == Some("most") => (FilterOp::Le, join_tokens(tokens, p + 2, end), end),
        ">" => (FilterOp::Gt, join_tokens(tokens, p + 1, end), end),
        ">=" => (FilterOp::Ge, join_tokens(tokens, p + 1, end), end),
        "<" => (FilterOp::Lt, join_tokens(tokens, p + 1, end), end),
        "<=" => (FilterOp::Le, join_tokens(tokens, p + 1, end), end),
        _ => return None,
    };
    if endpos != end || (op.needs_value() && value.trim().is_empty()) {
        return None;
    }
    let op = if negate { row_scope_negate(op)? } else { op };
    Some((op, value))
}

/// Flip an operator for a negated row-scope phrase (`no column is empty`
/// means `all notempty`). Prefix/suffix operators have no negation here.
fn row_scope_negate(op: FilterOp) -> Option<FilterOp> {
    Some(match op {
        FilterOp::Empty => FilterOp::NonEmpty,
        FilterOp::NonEmpty => FilterOp::Empty,
        FilterOp::Contains => FilterOp::NotContains,
        FilterOp::NotContains => FilterOp::Contains,
        FilterOp::Eq => FilterOp::Ne,
        FilterOp::Ne => FilterOp::Eq,
        _ => return None,
    })
}

#[derive(Debug, Clone)]
struct Token {
    text: String,
    lower: String,
}

/// Split on whitespace, keeping a double-quoted run as one token. A
/// trailing comma is a separator, so it becomes an implicit `and`.
fn tokenize(phrase: &str) -> Vec<Token> {
    let bytes = phrase.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            i += 1;
            let mut text = String::new();
            while i < bytes.len() && bytes[i] != b'"' {
                let ch = phrase[i..].chars().next().expect("non-empty remainder");
                text.push(ch);
                i += ch.len_utf8();
            }
            if i < bytes.len() {
                i += 1;
            }
            if !text.is_empty() {
                out.push(Token {
                    lower: text.to_ascii_lowercase(),
                    text,
                });
            }
            continue;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut text = phrase[start..i].to_string();
        let comma = text.ends_with(',');
        if comma {
            text.pop();
        }
        if !text.is_empty() {
            out.push(Token {
                lower: text.to_ascii_lowercase(),
                text,
            });
            if comma {
                out.push(Token {
                    text: "and".to_string(),
                    lower: "and".to_string(),
                });
            }
        }
    }
    out
}

/// Lowercased column name (and `colN` label, and alias) to index.
fn column_lookup(schema: &NlSchema) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    for (i, column) in schema.columns.iter().enumerate() {
        let key = column.name.to_ascii_lowercase();
        if !key.is_empty() {
            map.entry(key).or_insert(i);
        }
        map.entry(format!("col{i}")).or_insert(i);
    }
    for (alias, index) in &schema.aliases {
        if *index < schema.columns.len() {
            map.entry(alias.to_ascii_lowercase()).or_insert(*index);
        }
    }
    map
}

/// Longest window of up to four tokens that matches a column name.
fn match_column(
    tokens: &[Token],
    pos: usize,
    lookup: &HashMap<String, usize>,
) -> Option<(usize, usize)> {
    let max_window = 4.min(tokens.len() - pos);
    let mut best: Option<(usize, usize)> = None;
    for len in 1..=max_window {
        let phrase = tokens[pos..pos + len]
            .iter()
            .map(|t| t.lower.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(&index) = lookup.get(&phrase) {
            let better = match &best {
                None => true,
                Some((_, best_len)) => len > *best_len,
            };
            if better {
                best = Some((index, len));
            }
        }
    }
    best
}

/// Try to resolve a single token to a column by edit distance, as a
/// fallback for a small typo (`contry` -> `country`). Returns `None`
/// when there is no close match or when two columns tie.
fn fuzzy_column(
    tokens: &[Token],
    pos: usize,
    lookup: &HashMap<String, usize>,
) -> Option<(usize, usize)> {
    let token = &tokens[pos].lower;
    if token.len() < 3 || token.contains(' ') {
        return None;
    }
    let max_distance = if token.len() >= 8 { 2 } else { 1 };
    let mut best: Option<(usize, usize)> = None;
    let mut ambiguous = false;
    for (name, &index) in lookup {
        // Generated `colN` labels are not real names; skip them.
        if name
            .strip_prefix("col")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        {
            continue;
        }
        let distance = levenshtein(token, name);
        if distance > max_distance {
            continue;
        }
        match best {
            None => {
                best = Some((distance, index));
                ambiguous = false;
            }
            Some((best_distance, _)) if distance < best_distance => {
                best = Some((distance, index));
                ambiguous = false;
            }
            Some((best_distance, best_index))
                if distance == best_distance && index != best_index =>
            {
                ambiguous = true;
            }
            _ => {}
        }
    }
    if ambiguous {
        None
    } else {
        best.map(|(_, index)| (index, 1))
    }
}

/// Plain Levenshtein edit distance, `O(a.len() * b.len())`.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Leading words that carry no filter meaning ("show me", "list all").
fn is_filler(word: &str) -> bool {
    matches!(
        word,
        "show"
            | "list"
            | "find"
            | "display"
            | "give"
            | "me"
            | "all"
            | "the"
            | "records"
            | "rows"
            | "entries"
            | "where"
            | "with"
            | "that"
            | "which"
            | "whose"
            | "how"
            | "many"
            | "please"
            | "want"
            | "see"
            | "filter"
            | "any"
            | "some"
            | "there"
    )
}

fn join_tokens(tokens: &[Token], start: usize, end: usize) -> String {
    tokens[start..end]
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse `between A and B` into `column >= A AND column <= B`.
fn parse_between<F>(
    tokens: &[Token],
    pos: usize,
    column: usize,
    values: &F,
) -> Option<(FilterExpr, usize, bool)>
where
    F: Fn(usize) -> Vec<String>,
{
    if tokens.get(pos).map(|t| t.lower.as_str()) != Some("between") {
        return None;
    }
    let mut p = pos + 1;

    let (low, low_next, low_bound) = bind_bound(tokens, p, column, values, true)?;
    p = low_next;
    if tokens.get(p).map(|t| t.lower.as_str()) != Some("and") {
        return None;
    }
    p += 1;
    let (high, high_next, high_bound) = bind_bound(tokens, p, column, values, false)?;

    let expr = FilterExpr::And(
        Box::new(FilterExpr::Predicate(Filter {
            column,
            op: FilterOp::Ge,
            value: low,
        })),
        Box::new(FilterExpr::Predicate(Filter {
            column,
            op: FilterOp::Le,
            value: high,
        })),
    );
    Some((expr, high_next, low_bound && high_bound))
}

/// Bind one side of a `between` range. When `stop_at_and` is set the raw
/// fallback stops before `and`; otherwise it stops at a conjunction or the
/// end. Returns `(value, next, confident)`.
fn bind_bound<F>(
    tokens: &[Token],
    pos: usize,
    column: usize,
    values: &F,
    stop_at_and: bool,
) -> Option<(String, usize, bool)>
where
    F: Fn(usize) -> Vec<String>,
{
    if pos >= tokens.len() {
        return None;
    }
    if let Some((value, next)) = value_only(tokens, pos, column, values) {
        return Some((value, next, true));
    }
    if tokens[pos].text.parse::<f64>().is_ok() {
        return Some((tokens[pos].text.clone(), pos + 1, true));
    }
    let start = pos;
    let mut end = pos;
    while end < tokens.len()
        && conjunction_of(&tokens[end].lower).is_none()
        && !(stop_at_and && tokens[end].lower == "and")
    {
        end += 1;
    }
    if end == start {
        return None;
    }
    Some((join_tokens(tokens, start, end), end, false))
}

/// Words that can sit between a column and its operator.
fn is_linker(word: &str) -> bool {
    matches!(
        word,
        "is" | "are" | "was" | "were" | "be" | "being" | "the" | "a" | "an" | "of"
    )
}

/// Operator phrases, longest first, so `is not empty` beats `is`.
fn operator_phrases() -> Vec<(&'static str, FilterOp)> {
    let mut phrases: Vec<(&'static str, FilterOp)> = OPERATOR_PHRASES.to_vec();
    phrases.sort_by(|a, b| {
        let a_words = a.0.split(' ').count();
        let b_words = b.0.split(' ').count();
        b_words
            .cmp(&a_words)
            .then_with(|| b.0.len().cmp(&a.0.len()))
    });
    phrases
}

/// Match an operator at `pos`, skipping up to two linking words.
fn match_operator(
    tokens: &[Token],
    pos: usize,
    phrases: &[(&'static str, FilterOp)],
) -> Option<(FilterOp, usize)> {
    let mut p = pos;
    for _ in 0..3 {
        for (phrase, op) in phrases {
            let words: Vec<&str> = phrase.split(' ').collect();
            if p + words.len() <= tokens.len()
                && (0..words.len()).all(|k| tokens[p + k].lower == words[k])
            {
                return Some((*op, (p - pos) + words.len()));
            }
        }
        if p < tokens.len() && is_linker(&tokens[p].lower) {
            p += 1;
        } else {
            break;
        }
    }
    None
}

/// Bind the value for `op`, preferring the longest sampled value for the
/// column, then a numeric literal, then the raw remaining words (which
/// are reported as low confidence via `bound == false`).
fn bind_value<F>(
    tokens: &[Token],
    pos: usize,
    op: FilterOp,
    column: usize,
    values: &F,
) -> (String, usize, bool)
where
    F: Fn(usize) -> Vec<String>,
{
    if !op.needs_value() {
        return (String::new(), pos, true);
    }
    if pos >= tokens.len() {
        return (String::new(), pos, false);
    }
    if let Some((value, next)) = value_only(tokens, pos, column, values) {
        return (value, next, true);
    }
    if op.is_numeric() && tokens[pos].text.parse::<f64>().is_ok() {
        return (tokens[pos].text.clone(), pos + 1, true);
    }
    // Consume until the next conjunction.
    let mut end = pos;
    while end < tokens.len() && conjunction_of(&tokens[end].lower).is_none() {
        end += 1;
    }
    let value = tokens[pos..end]
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    // Substring-style operators accept arbitrary text, so they do not
    // lose confidence; equality against an unknown value does.
    let confident = matches!(
        op,
        FilterOp::Contains | FilterOp::NotContains | FilterOp::StartsWith | FilterOp::EndsWith
    );
    (value, end, confident)
}

/// If the tokens at `pos` exactly match a sampled value for `column`,
/// return that value (original casing) and the index just past it.
fn value_only<F>(tokens: &[Token], pos: usize, column: usize, values: &F) -> Option<(String, usize)>
where
    F: Fn(usize) -> Vec<String>,
{
    if pos >= tokens.len() {
        return None;
    }
    let samples = values(column);
    let mut best: Option<(String, usize)> = None;
    for sample in samples {
        // A quoted multi-word value arrives as a single token.
        if tokens[pos].lower == sample.to_ascii_lowercase() {
            let better = match &best {
                None => true,
                Some((_, end)) => pos + 1 > *end,
            };
            if better {
                best = Some((sample.clone(), pos + 1));
            }
            continue;
        }
        let words: Vec<&str> = sample.split_whitespace().collect();
        if words.is_empty() || pos + words.len() > tokens.len() {
            continue;
        }
        let matches =
            (0..words.len()).all(|k| tokens[pos + k].lower == words[k].to_ascii_lowercase());
        let better = match &best {
            None => true,
            Some((_, end)) => pos + words.len() > *end,
        };
        if matches && better {
            best = Some((sample.clone(), pos + words.len()));
        }
    }
    best
}

/// `Some(true)` for AND, `Some(false)` for OR.
fn conjunction_of(word: &str) -> Option<bool> {
    match word {
        "and" | "also" | "plus" | "then" | "&" => Some(true),
        "or" | "|" => Some(false),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Tier 2: optional local-model backend
// ---------------------------------------------------------------------

/// An error from the optional Tier 2 backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    pub message: String,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A Tier 2 backend that shells out to a user-configured local command.
///
/// The command is a prompt-in / text-out filter: Anvil writes
/// [`model_prompt`] to its stdin and reads the last non-empty stdout line
/// as the candidate expression. Anvil never downloads a model and never
/// contacts the network itself; whatever the configured command does is
/// the user's choice. Output is always re-validated by [`interpret`]
/// before it can become a query, so a model cannot emit an arbitrary
/// expression. This works the same on macOS, Windows, and Linux: the
/// program is spawned directly, with no shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelBackend {
    program: String,
    args: Vec<String>,
}

impl ModelBackend {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    /// Run the backend on `phrase` and return its candidate expression
    /// (not yet validated).
    pub fn complete<F>(
        &self,
        phrase: &str,
        schema: &NlSchema,
        values: &F,
    ) -> Result<String, ModelError>
    where
        F: Fn(usize) -> Vec<String>,
    {
        let prompt = model_prompt(phrase, schema, values);
        run_command(&self.program, &self.args, &prompt)
    }
}

/// Translate `phrase` with `backend` and validate the result against the
/// real filter grammar. Model text is never trusted directly.
pub fn interpret<F>(
    phrase: &str,
    schema: &NlSchema,
    values: &F,
    backend: &ModelBackend,
) -> Result<FilterExpr, ModelError>
where
    F: Fn(usize) -> Vec<String>,
{
    let text = backend.complete(phrase, schema, values)?;
    validate_candidate(&text, schema)
}

/// Validate a candidate expression string against the schema with the
/// real filter parser. Every external backend (subprocess or embedded
/// model) goes through here, so no backend can emit an arbitrary query.
pub fn validate_candidate(text: &str, schema: &NlSchema) -> Result<FilterExpr, ModelError> {
    let headers: Vec<String> = schema.columns.iter().map(|c| c.name.clone()).collect();
    let column_count = schema.columns.len();
    super::filter::parse_expr(text, &headers, column_count).map_err(|err| ModelError {
        message: format!("backend returned an invalid expression: {err}"),
    })
}

/// Build the prompt sent to a Tier 2 backend: an instruction, the
/// grammar, the columns (with sample values only for columns named in
/// the phrase, so the prompt does not scan every column), and the
/// question.
pub fn model_prompt<F>(phrase: &str, schema: &NlSchema, values: &F) -> String
where
    F: Fn(usize) -> Vec<String>,
{
    const MAX_SAMPLES: usize = 20;
    const MAX_COLUMNS: usize = 200;
    let phrase_lower = phrase.to_ascii_lowercase();
    let mut out = String::new();
    out.push_str("Convert the question below into exactly one Anvil filter expression.\n");
    out.push_str("Rules:\n");
    out.push_str(
        "- Use only the column names listed below, exactly as written (keep any quotes).\n",
    );
    out.push_str("- Reply with the expression only: one line, no explanation, no code fences.\n");
    out.push_str(
        "- Operators: ==, !=, >, >=, <, <=, contains, !contains, startswith, endswith, empty, notempty.\n",
    );
    out.push_str("- Combine predicates with AND / OR and parentheses.\n");
    out.push_str("Examples:\n");
    out.push_str("- last names starting with C -> last_name startswith C\n");
    out.push_str("- US orders over 100 -> country == US AND amount > 100\n");
    out.push_str("- status is failed or refunded -> status == failed OR status == refunded\n");
    out.push_str("Columns:\n");
    for (i, column) in schema.columns.iter().enumerate().take(MAX_COLUMNS) {
        let type_name = column_type_name(column.column_type);
        // A name with spaces or punctuation is shown quoted, exactly how it
        // must appear in the expression.
        let name = display_column_name(&column.name);
        let mentioned = phrase_lower.contains(&column.name.to_ascii_lowercase());
        if mentioned {
            let samples = values(i);
            let shown: Vec<&str> = samples
                .iter()
                .take(MAX_SAMPLES)
                .map(String::as_str)
                .collect();
            if shown.is_empty() {
                out.push_str(&format!("- {name} ({type_name})\n"));
            } else {
                out.push_str(&format!("- {name} ({type_name}): {}\n", shown.join(", ")));
            }
        } else {
            out.push_str(&format!("- {name} ({type_name})\n"));
        }
    }
    out.push_str(&format!("Question: {phrase}\nExpression:"));
    out
}

/// Render a column name for the prompt, quoting it when it is not a plain
/// identifier so the model copies it verbatim.
fn display_column_name(name: &str) -> String {
    if name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        && !name.is_empty()
    {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\\\""))
    }
}

fn column_type_name(column_type: ColumnType) -> &'static str {
    match column_type {
        ColumnType::Integer | ColumnType::Float => "number",
        ColumnType::Boolean => "boolean",
        ColumnType::Text => "text",
        ColumnType::Empty => "empty",
    }
}

fn run_command(program: &str, args: &[String], prompt: &str) -> Result<String, ModelError> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| ModelError {
            message: format!("could not start `{program}`: {err}"),
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .map_err(|err| ModelError {
                message: format!("could not write to `{program}`: {err}"),
            })?;
        // Dropping stdin closes it so the child sees EOF.
    }
    let output = child.wait_with_output().map_err(|err| ModelError {
        message: format!("`{program}` failed: {err}"),
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    let cleaned = strip_fences(line);
    if cleaned.is_empty() {
        return Err(ModelError {
            message: "backend returned no output".to_string(),
        });
    }
    Ok(cleaned)
}

/// Remove a Markdown code fence or surrounding quotes a model may have
/// added around the expression.
pub fn strip_fences(line: &str) -> String {
    let mut text = line.trim();
    if let Some(rest) = text.strip_prefix("```") {
        let rest = rest.split_once('\n').map(|(_, body)| body).unwrap_or(rest);
        text = rest.trim();
        if let Some(rest) = text.strip_suffix("```") {
            text = rest.trim();
        }
    }
    text.trim_matches(|c| c == '"' || c == '`')
        .trim()
        .to_string()
}

/// Render a parsed expression back to filter-bar syntax, quoting column
/// names and values that contain whitespace or quotes.
pub fn describe(expr: &FilterExpr, headers: &[String]) -> String {
    match expr {
        FilterExpr::Predicate(filter) => describe_predicate(filter, headers),
        // `any` / `all` scope the predicate across every column, so the
        // pseudo-column is rendered verbatim (round-trips through parse_expr).
        FilterExpr::AnyColumn(filter) => describe_row_scope("any", filter),
        FilterExpr::AllColumn(filter) => describe_row_scope("all", filter),
        FilterExpr::And(a, b) => format!("{} AND {}", describe(a, headers), describe(b, headers)),
        FilterExpr::Or(a, b) => format!("({} OR {})", describe(a, headers), describe(b, headers)),
    }
}

fn describe_row_scope(scope: &str, filter: &Filter) -> String {
    let op = op_symbol(filter.op);
    if filter.op.needs_value() {
        format!("{scope} {op} {}", quote_if_needed(&filter.value))
    } else {
        format!("{scope} {op}")
    }
}

fn describe_predicate(filter: &Filter, headers: &[String]) -> String {
    let column = quote_if_needed(&crate::parser::column_label(headers, filter.column));
    let op = op_symbol(filter.op);
    if filter.op.needs_value() {
        format!("{column} {op} {}", quote_if_needed(&filter.value))
    } else {
        format!("{column} {op}")
    }
}

fn quote_if_needed(text: &str) -> String {
    if text.chars().any(char::is_whitespace) || text.contains('"') {
        format!("\"{}\"", text.replace('"', "\\\""))
    } else {
        text.to_string()
    }
}

fn op_symbol(op: FilterOp) -> &'static str {
    match op {
        FilterOp::Eq => "==",
        FilterOp::Ne => "!=",
        FilterOp::Contains => "contains",
        FilterOp::NotContains => "!contains",
        FilterOp::StartsWith => "startswith",
        FilterOp::EndsWith => "endswith",
        FilterOp::Empty => "empty",
        FilterOp::NonEmpty => "notempty",
        FilterOp::Gt => ">",
        FilterOp::Ge => ">=",
        FilterOp::Lt => "<",
        FilterOp::Le => "<=",
    }
}

const OPERATOR_PHRASES: &[(&str, FilterOp)] = &[
    ("is not empty", FilterOp::NonEmpty),
    ("is not blank", FilterOp::NonEmpty),
    ("is not missing", FilterOp::NonEmpty),
    ("is not null", FilterOp::NonEmpty),
    ("is greater than or equal to", FilterOp::Ge),
    ("is less than or equal to", FilterOp::Le),
    ("is greater than", FilterOp::Gt),
    ("is less than", FilterOp::Lt),
    ("is at least", FilterOp::Ge),
    ("is at most", FilterOp::Le),
    ("is over", FilterOp::Gt),
    ("is under", FilterOp::Lt),
    ("is above", FilterOp::Gt),
    ("is below", FilterOp::Lt),
    ("is not", FilterOp::Ne),
    ("is empty", FilterOp::Empty),
    ("is blank", FilterOp::Empty),
    ("is missing", FilterOp::Empty),
    ("is null", FilterOp::Empty),
    ("does not contain", FilterOp::NotContains),
    ("doesn't contain", FilterOp::NotContains),
    ("not contains", FilterOp::NotContains),
    ("not empty", FilterOp::NonEmpty),
    ("not blank", FilterOp::NonEmpty),
    ("greater than or equal to", FilterOp::Ge),
    ("less than or equal to", FilterOp::Le),
    ("greater than", FilterOp::Gt),
    ("more than", FilterOp::Gt),
    ("less than", FilterOp::Lt),
    ("fewer than", FilterOp::Lt),
    ("at least", FilterOp::Ge),
    ("at most", FilterOp::Le),
    ("equal to", FilterOp::Eq),
    ("not equal to", FilterOp::Ne),
    ("starts with", FilterOp::StartsWith),
    ("begins with", FilterOp::StartsWith),
    ("starting with", FilterOp::StartsWith),
    ("ends with", FilterOp::EndsWith),
    ("has a value", FilterOp::NonEmpty),
    ("contains", FilterOp::Contains),
    ("containing", FilterOp::Contains),
    ("includes", FilterOp::Contains),
    ("including", FilterOp::Contains),
    ("without", FilterOp::NotContains),
    ("excluding", FilterOp::NotContains),
    ("empty", FilterOp::Empty),
    ("blank", FilterOp::Empty),
    ("missing", FilterOp::Empty),
    ("present", FilterOp::NonEmpty),
    ("equals", FilterOp::Eq),
    ("over", FilterOp::Gt),
    ("above", FilterOp::Gt),
    ("exceeds", FilterOp::Gt),
    ("under", FilterOp::Lt),
    ("below", FilterOp::Lt),
    ("has", FilterOp::Contains),
    ("is", FilterOp::Eq),
    ("=", FilterOp::Eq),
    ("==", FilterOp::Eq),
    ("!=", FilterOp::Ne),
    (">=", FilterOp::Ge),
    ("<=", FilterOp::Le),
    (">", FilterOp::Gt),
    ("<", FilterOp::Lt),
    ("no more than", FilterOp::Le),
    ("no less than", FilterOp::Ge),
    ("no greater than", FilterOp::Le),
    ("no fewer than", FilterOp::Ge),
    ("at minimum", FilterOp::Ge),
    ("at maximum", FilterOp::Le),
    ("matches", FilterOp::Contains),
    ("contain", FilterOp::Contains),
    ("like", FilterOp::Contains),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> NlSchema {
        NlSchema {
            columns: vec![
                NlColumn {
                    name: "country".into(),
                    column_type: ColumnType::Text,
                },
                NlColumn {
                    name: "amount".into(),
                    column_type: ColumnType::Float,
                },
                NlColumn {
                    name: "status".into(),
                    column_type: ColumnType::Text,
                },
                NlColumn {
                    name: "note".into(),
                    column_type: ColumnType::Text,
                },
            ],
            aliases: vec![("total".into(), 1)],
        }
    }

    #[test]
    fn describe_round_trips_row_scope_predicates() {
        let headers = vec!["country".to_string(), "amount".to_string()];
        for text in [
            "any empty",
            "all notempty",
            "any contains foo",
            "any > 5",
            "(any empty) OR (country == US)",
            "all notempty AND any empty",
        ] {
            let parsed = crate::query::filter::parse_expr(text, &headers, 2).unwrap();
            let rendered = describe(&parsed, &headers);
            let reparsed = crate::query::filter::parse_expr(&rendered, &headers, 2).unwrap();
            assert_eq!(parsed, reparsed, "{text} -> {rendered}");
        }
    }

    fn values(column: usize) -> Vec<String> {
        match column {
            0 => vec![
                "US".to_string(),
                "UK".to_string(),
                "Madagascar".to_string(),
                "Trinidad and Tobago".to_string(),
            ],
            2 => vec![
                "failed".to_string(),
                "settled".to_string(),
                "refunded".to_string(),
            ],
            _ => Vec::new(),
        }
    }

    fn run(phrase: &str) -> NlResult {
        parse(phrase, &schema(), &values)
    }

    #[test]
    fn parses_row_scope_phrases() {
        let cases: &[(&str, bool, FilterOp, &str)] = &[
            ("at least one column is blank", true, FilterOp::Empty, ""),
            (
                "showing entries where at least 1 column is blank",
                true,
                FilterOp::Empty,
                "",
            ),
            ("any column is empty", true, FilterOp::Empty, ""),
            ("some field is blank", true, FilterOp::Empty, ""),
            ("every column is filled", false, FilterOp::NonEmpty, ""),
            ("all columns are not empty", false, FilterOp::NonEmpty, ""),
            ("no column is empty", false, FilterOp::NonEmpty, ""),
            ("all columns are empty", false, FilterOp::Empty, ""),
            ("any column contains foo", true, FilterOp::Contains, "foo"),
            ("all columns contain foo", false, FilterOp::Contains, "foo"),
        ];
        for (phrase, any, op, value) in cases {
            let result = run(phrase);
            let filter = Filter {
                column: 0,
                op: *op,
                value: (*value).into(),
            };
            let expected = if *any {
                FilterExpr::AnyColumn(filter)
            } else {
                FilterExpr::AllColumn(filter)
            };
            assert_eq!(result.expr, Some(expected), "{phrase}");
            assert!(result.clean, "{phrase} should be clean");
            assert!(
                result.unmatched.is_empty(),
                "{phrase}: {:?}",
                result.unmatched
            );
        }
    }

    #[test]
    fn parses_row_has_column_phrases() {
        let cases: &[(&str, bool, FilterOp)] = &[
            ("Any row that has an empty column", false, FilterOp::Empty),
            ("rows with an empty cell", false, FilterOp::Empty),
            ("any row that has no empty column", true, FilterOp::NonEmpty),
            ("all rows that have a blank field", false, FilterOp::Empty),
            ("no row that has a filled value", true, FilterOp::Empty),
        ];
        for (phrase, all, op) in cases {
            let result = run(phrase);
            let filter = Filter {
                column: 0,
                op: *op,
                value: String::new(),
            };
            let expected = if *all {
                FilterExpr::AllColumn(filter)
            } else {
                FilterExpr::AnyColumn(filter)
            };
            assert_eq!(result.expr, Some(expected), "{phrase}");
            assert!(result.clean, "{phrase} should be clean");
            assert!(
                result.unmatched.is_empty(),
                "{phrase}: {:?}",
                result.unmatched
            );
        }
    }

    #[test]
    fn row_has_phrases_do_not_hijack_column_filters() {
        for phrase in [
            "list all rows with amount over 100",
            "rows where country is US",
        ] {
            let result = run(phrase);
            assert!(
                matches!(result.expr, Some(FilterExpr::Predicate(_))),
                "{phrase}: {:?}",
                result.expr
            );
        }
    }

    #[test]
    fn row_scope_phrases_do_not_hijack_column_phrases() {
        // `any` / `all` are filler before a real column, not scope markers.
        for phrase in ["any amount over 100", "show me country is US"] {
            let result = run(phrase);
            assert!(
                matches!(result.expr, Some(FilterExpr::Predicate(_))),
                "{phrase}: {:?}",
                result.expr
            );
        }
    }

    #[test]
    fn rules_row_scope_round_trips_through_describe() {
        let headers = vec!["country".to_string(), "amount".to_string()];
        for phrase in [
            "at least one column is blank",
            "every column is filled",
            "any column contains foo",
        ] {
            let expr = run(phrase).expr.expect(phrase);
            let rendered = describe(&expr, &headers);
            let reparsed =
                crate::query::filter::parse_expr(&rendered, &headers, headers.len()).unwrap();
            assert_eq!(expr, reparsed, "{phrase} -> {rendered}");
        }
    }

    fn predicate(column: usize, op: FilterOp, value: &str) -> FilterExpr {
        FilterExpr::Predicate(Filter {
            column,
            op,
            value: value.to_string(),
        })
    }

    #[test]
    fn explicit_equality_parses_cleanly() {
        let r = run("country is US");
        assert!(r.clean, "{r:?}");
        assert_eq!(r.expr, Some(predicate(0, FilterOp::Eq, "US")));
        assert!(r.unmatched.is_empty());
    }

    #[test]
    fn symbolic_operator_parses() {
        assert_eq!(
            run("amount > 100").expr,
            Some(predicate(1, FilterOp::Gt, "100"))
        );
        assert_eq!(
            run("amount >= 100").expr,
            Some(predicate(1, FilterOp::Ge, "100"))
        );
    }

    #[test]
    fn numeric_comparison_phrases() {
        assert_eq!(
            run("amount over 100").expr,
            Some(predicate(1, FilterOp::Gt, "100"))
        );
        assert_eq!(
            run("amount is greater than 100").expr,
            Some(predicate(1, FilterOp::Gt, "100"))
        );
        assert_eq!(
            run("amount at least 50").expr,
            Some(predicate(1, FilterOp::Ge, "50"))
        );
        assert_eq!(
            run("amount is at most 10").expr,
            Some(predicate(1, FilterOp::Le, "10"))
        );
    }

    #[test]
    fn negation_maps_to_negating_operators() {
        assert_eq!(
            run("country is not US").expr,
            Some(predicate(0, FilterOp::Ne, "US"))
        );
        assert_eq!(
            run("note not empty").expr,
            Some(predicate(3, FilterOp::NonEmpty, ""))
        );
        assert_eq!(
            run("note is empty").expr,
            Some(predicate(3, FilterOp::Empty, ""))
        );
    }

    #[test]
    fn contains_accepts_an_unsampled_prefix_without_losing_confidence() {
        let r = run("status contains fail");
        assert!(r.clean, "{r:?}");
        assert_eq!(r.expr, Some(predicate(2, FilterOp::Contains, "fail")));
    }

    #[test]
    fn equality_against_an_unknown_value_is_low_confidence() {
        let r = run("country is Atlantis");
        assert!(!r.clean, "{r:?}");
        assert_eq!(r.expr, Some(predicate(0, FilterOp::Eq, "Atlantis")));
        assert!(r.unmatched.contains(&"Atlantis".to_string()));
    }

    #[test]
    fn conjunction_and_or_combine() {
        assert_eq!(
            run("country is US and amount over 100").expr,
            Some(FilterExpr::And(
                Box::new(predicate(0, FilterOp::Eq, "US")),
                Box::new(predicate(1, FilterOp::Gt, "100")),
            ))
        );
        assert_eq!(
            run("status is failed or refunded").expr,
            Some(FilterExpr::Or(
                Box::new(predicate(2, FilterOp::Eq, "failed")),
                Box::new(predicate(2, FilterOp::Eq, "refunded")),
            ))
        );
    }

    #[test]
    fn multi_word_sampled_value_wins_over_a_conjunction() {
        // `Trinidad and Tobago` is one sampled value, so the `and` inside
        // it must not split the predicate.
        let r = run("country is Trinidad and Tobago");
        assert!(r.clean, "{r:?}");
        assert_eq!(
            r.expr,
            Some(predicate(0, FilterOp::Eq, "Trinidad and Tobago"))
        );
    }

    #[test]
    fn multi_word_column_name_matches() {
        let schema = NlSchema {
            columns: vec![
                NlColumn {
                    name: "New York".into(),
                    column_type: ColumnType::Text,
                },
                NlColumn {
                    name: "pad".into(),
                    column_type: ColumnType::Text,
                },
            ],
            aliases: vec![],
        };
        let values = |_col: usize| vec!["NYC".to_string()];
        let r = parse("\"New York\" is NYC", &schema, &values);
        assert!(r.clean, "{r:?}");
        assert_eq!(r.expr, Some(predicate(0, FilterOp::Eq, "NYC")));
    }

    #[test]
    fn alias_resolves_to_its_column() {
        let r = run("total over 100");
        assert!(r.clean, "{r:?}");
        assert_eq!(r.expr, Some(predicate(1, FilterOp::Gt, "100")));
    }

    #[test]
    fn comma_acts_as_and() {
        assert_eq!(
            run("country is US, amount over 100").expr,
            Some(FilterExpr::And(
                Box::new(predicate(0, FilterOp::Eq, "US")),
                Box::new(predicate(1, FilterOp::Gt, "100")),
            ))
        );
    }

    #[test]
    fn unknown_words_are_reported_and_not_clean() {
        let r = run("banana");
        assert!(r.expr.is_none());
        assert!(!r.clean);
        assert_eq!(r.unmatched, vec!["banana".to_string()]);
    }

    #[test]
    fn unknown_filler_after_a_predicate_is_unclean() {
        let r = run("country is US banana");
        assert!(!r.clean);
        assert!(r.unmatched.contains(&"banana".to_string()));
    }

    #[test]
    fn empty_phrase_is_not_clean() {
        let r = run("   ");
        assert!(r.expr.is_none());
        assert!(!r.clean);
    }

    #[test]
    fn quoted_value_is_one_token() {
        let r = run("country is \"Trinidad and Tobago\"");
        assert!(r.clean, "{r:?}");
        assert_eq!(
            r.expr,
            Some(predicate(0, FilterOp::Eq, "Trinidad and Tobago"))
        );
    }

    #[test]
    fn describe_round_trips_back_to_filter_syntax() {
        let expr = FilterExpr::And(
            Box::new(predicate(0, FilterOp::Eq, "US")),
            Box::new(predicate(1, FilterOp::Gt, "100")),
        );
        let headers = vec!["country".to_string(), "amount".to_string()];
        assert_eq!(describe(&expr, &headers), "country == US AND amount > 100");
        // A multi-word value is quoted so it stays one token.
        let expr = predicate(0, FilterOp::Eq, "Trinidad and Tobago");
        assert_eq!(
            describe(&expr, &headers),
            "country == \"Trinidad and Tobago\""
        );
    }

    #[test]
    fn filler_words_and_synonyms_are_handled() {
        let r = run("show me all the rows where country is US");
        assert!(r.clean, "{r:?}");
        assert_eq!(r.expr, Some(predicate(0, FilterOp::Eq, "US")));
        assert_eq!(
            run("amount no more than 10").expr,
            Some(predicate(1, FilterOp::Le, "10"))
        );
        assert_eq!(
            run("status matches fail").expr,
            Some(predicate(2, FilterOp::Contains, "fail"))
        );
    }

    #[test]
    fn fuzzy_column_name_matches_a_typo() {
        let r = run("contry is US");
        assert!(r.clean, "{r:?}");
        assert_eq!(r.expr, Some(predicate(0, FilterOp::Eq, "US")));
    }

    #[test]
    fn between_range_expands_to_ge_and_le() {
        let r = run("amount between 10 and 20");
        assert!(r.clean, "{r:?}");
        assert_eq!(
            r.expr,
            Some(FilterExpr::And(
                Box::new(predicate(1, FilterOp::Ge, "10")),
                Box::new(predicate(1, FilterOp::Le, "20")),
            ))
        );
    }

    #[test]
    fn model_prompt_quotes_spaced_column_names() {
        let schema = NlSchema {
            columns: vec![
                NlColumn {
                    name: "Last Name".into(),
                    column_type: ColumnType::Text,
                },
                NlColumn {
                    name: "pad".into(),
                    column_type: ColumnType::Text,
                },
            ],
            aliases: vec![],
        };
        let prompt = model_prompt("last name starts with C", &schema, &|_| Vec::new());
        assert!(prompt.contains("- \"Last Name\" (text)"), "{prompt}");
    }

    #[test]
    fn model_prompt_samples_only_columns_named_in_the_phrase() {
        let prompt = model_prompt("country is US", &schema(), &values);
        assert!(
            prompt.contains("country (text): US, UK, Madagascar, Trinidad and Tobago"),
            "{prompt}"
        );
        // `amount` is not named, so it is listed without samples.
        assert!(prompt.contains("- amount (number)\n"), "{prompt}");
        assert!(prompt.contains("Question: country is US"), "{prompt}");
    }

    #[test]
    fn strip_fences_removes_markdown_and_quotes() {
        assert_eq!(strip_fences("```country == US```"), "country == US");
        assert_eq!(strip_fences("`country == US`"), "country == US");
        assert_eq!(strip_fences("\"country == US\""), "country == US");
        assert_eq!(strip_fences("```text\ncountry == US\n```"), "country == US");
    }

    #[cfg(unix)]
    #[test]
    fn model_backend_runs_a_command_and_returns_the_last_line() {
        let backend = ModelBackend::new(
            "sh",
            vec![
                "-c".to_string(),
                "printf 'ignore me\\ncountry == US AND amount > 100\\n'".to_string(),
            ],
        );
        let text = backend
            .complete("anything", &schema(), &values)
            .expect("complete");
        assert_eq!(text, "country == US AND amount > 100");
    }

    #[cfg(unix)]
    #[test]
    fn interpret_validates_model_output_through_the_filter_parser() {
        let good = ModelBackend::new(
            "sh",
            vec!["-c".to_string(), "printf 'country == US\\n'".to_string()],
        );
        let expr = interpret("who is in the US", &schema(), &values, &good).expect("valid");
        assert_eq!(expr, predicate(0, FilterOp::Eq, "US"));

        // Garbage from the model must not become a query.
        let bad = ModelBackend::new(
            "sh",
            vec![
                "-c".to_string(),
                "printf 'this is not a filter\\n'".to_string(),
            ],
        );
        assert!(interpret("who is in the US", &schema(), &values, &bad).is_err());

        // A missing program is reported, not panicked.
        let missing = ModelBackend::new("definitely-not-a-real-program-anvil", Vec::new());
        assert!(interpret("x", &schema(), &values, &missing).is_err());
    }
}

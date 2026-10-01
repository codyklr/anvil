//! Filter-input autocomplete context.
//!
//! While the user types a filter expression we need to know which part
//! of the grammar the caret is in so we can offer the right suggestions
//! (a column, an operator, a value, or the boolean keywords). This is a
//! lenient state machine: unlike [`super::filter::parse_expr`] it never
//! fails, because it runs on every keystroke over an incomplete
//! expression.
//!
//! It is GPUI-free and unit-testable; the UI supplies the value
//! candidates (a bounded sample) and renders the popup.

use std::ops::Range;

use crate::parser::column_label;

use super::filter::{resolve_column, FilterOp};

/// Operator spellings offered as suggestions.
pub const OPERATORS: &[&str] = &[
    "==",
    "!=",
    ">",
    ">=",
    "<",
    "<=",
    "contains",
    "!contains",
    "startswith",
    "endswith",
    "empty",
    "notempty",
];

/// Boolean keywords offered after a complete predicate.
pub const KEYWORDS: &[&str] = &["AND", "OR"];

/// Maximum suggestions returned for one slot.
pub const MAX_SUGGESTIONS: usize = 64;

/// Which grammar slot the caret is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// A column name / index / `#N`.
    Column,
    /// An operator, for the resolved column when there is one.
    Operator { column: Option<usize> },
    /// A value for `column` under `op`.
    Value { column: Option<usize>, op: FilterOp },
    /// The boolean keywords after a complete predicate.
    Keyword,
}

/// The slot at `caret`, the byte range a suggestion should replace, and
/// the partial text used to filter candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestContext {
    pub slot: Slot,
    /// Byte range of the token under the caret, or an empty range at
    /// `caret` when the slot has no partial token yet.
    pub replace: Range<usize>,
    /// What has already been typed for this slot (empty at a fresh slot).
    pub partial: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokKind {
    Word,
    LParen,
    RParen,
}

#[derive(Debug, Clone)]
struct Tok {
    kind: TokKind,
    start: usize,
    end: usize,
    /// Unquoted text for words.
    text: String,
}

/// Lenient token scanner. `(` and `)` are standalone; a double-quoted run
/// is one word (quotes stripped, `\"` unescaped); an unterminated quote
/// simply ends at the input end. Never fails.
fn scan(input: &str) -> Vec<Tok> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b if b.is_ascii_whitespace() => i += 1,
            b'(' => {
                out.push(Tok {
                    kind: TokKind::LParen,
                    start: i,
                    end: i + 1,
                    text: "(".into(),
                });
                i += 1;
            }
            b')' => {
                out.push(Tok {
                    kind: TokKind::RParen,
                    start: i,
                    end: i + 1,
                    text: ")".into(),
                });
                i += 1;
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut text = String::new();
                while i < bytes.len() {
                    if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'"') {
                        text.push('"');
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    let ch = input[i..].chars().next().expect("non-empty");
                    text.push(ch);
                    i += ch.len_utf8();
                }
                out.push(Tok {
                    kind: TokKind::Word,
                    start,
                    end: i,
                    text,
                });
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
                out.push(Tok {
                    kind: TokKind::Word,
                    start,
                    end: i,
                    text: input[start..i].to_string(),
                });
            }
        }
    }
    out
}

fn is_keyword(text: &str) -> bool {
    text.eq_ignore_ascii_case("and") || text.eq_ignore_ascii_case("or")
}

/// Lenient parser state. Mirrors the grammar but treats a missing or
/// malformed operator as "still typing it".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Column,
    Operator { column: Option<usize> },
    Value { column: Option<usize>, op: FilterOp },
    Keyword,
}

/// Work out the slot at `caret`.
pub fn context(
    input: &str,
    caret: usize,
    headers: &[String],
    column_count: usize,
) -> SuggestContext {
    let caret = caret.min(input.len());
    let tokens = scan(input);

    // The token the caret sits in, if any (only words are editable).
    let caret_idx = tokens
        .iter()
        .position(|tok| tok.kind == TokKind::Word && caret >= tok.start && caret <= tok.end);

    // Tokens fully before the caret drive the state machine. When the
    // caret is in whitespace (or at the very end after a space) all
    // tokens ending at or before the caret are complete.
    let complete = match caret_idx {
        Some(i) => i,
        None => tokens.iter().take_while(|tok| tok.end <= caret).count(),
    };

    let mut state = State::Column;
    for tok in &tokens[..complete] {
        match tok.kind {
            TokKind::Word => match state {
                State::Column => {
                    state = State::Operator {
                        column: resolve_column(&tok.text, headers, column_count).ok(),
                    };
                }
                State::Operator { column } => {
                    if let Some(op) = FilterOp::parse(&tok.text) {
                        state = if op.needs_value() {
                            State::Value { column, op }
                        } else {
                            State::Keyword
                        };
                    }
                }
                State::Value { .. } => {
                    // A value may span several words; a boolean keyword
                    // ends it and opens a new predicate.
                    if is_keyword(&tok.text) {
                        state = State::Column;
                    }
                }
                State::Keyword => {}
            },
            TokKind::LParen => state = State::Column,
            TokKind::RParen => state = State::Keyword,
        }
    }

    let slot = match state {
        State::Column => Slot::Column,
        State::Operator { column } => Slot::Operator { column },
        State::Value { column, op } => Slot::Value { column, op },
        State::Keyword => Slot::Keyword,
    };

    let (partial, replace) = match caret_idx {
        Some(i) => {
            let tok = &tokens[i];
            (strip_quotes(&input[tok.start..caret]), tok.start..tok.end)
        }
        None => (String::new(), caret..caret),
    };

    SuggestContext {
        slot,
        replace,
        partial,
    }
}

/// Remove surrounding double quotes from a partial token.
fn strip_quotes(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix('"').unwrap_or(t);
    let t = t.strip_suffix('"').unwrap_or(t);
    t.to_string()
}

/// Candidate strings for `ctx`, case-insensitively prefix-filtered by
/// `ctx.partial`, de-duplicated, and capped at [`MAX_SUGGESTIONS`].
///
/// `values` are the (bounded) distinct values for the value slot's
/// column, supplied by the caller so this stays allocation-light and
/// independent of sampling.
pub fn candidates(
    ctx: &SuggestContext,
    headers: &[String],
    column_count: usize,
    values: Option<&[String]>,
) -> Vec<String> {
    let partial_lower = ctx.partial.to_ascii_lowercase();
    // (match rank, insertion order, text); sorted best-first below.
    let mut scored: Vec<(u8, usize, String)> = Vec::new();
    match &ctx.slot {
        Slot::Column => {
            // `any` / `all` are row-scope pseudo-columns (`any empty`).
            // A real column literally named `any` or `all` is reachable
            // by `#N` instead.
            push_scored(&mut scored, &partial_lower, "any");
            push_scored(&mut scored, &partial_lower, "all");
            for i in 0..column_count.min(MAX_SUGGESTIONS * 4) {
                let label = column_label(headers, i);
                push_scored(&mut scored, &partial_lower, &label);
            }
        }
        Slot::Operator { .. } => {
            for op in OPERATORS {
                push_scored(&mut scored, &partial_lower, op);
            }
        }
        Slot::Value { .. } => {
            for kw in KEYWORDS {
                push_scored(&mut scored, &partial_lower, kw);
            }
            if let Some(values) = values {
                for value in values {
                    push_scored(&mut scored, &partial_lower, value);
                }
            }
        }
        Slot::Keyword => {
            for kw in KEYWORDS {
                push_scored(&mut scored, &partial_lower, kw);
            }
        }
    }
    // Whole-string prefix beats a word prefix, which beats a substring,
    // so the best match is at the top; ties keep first-seen order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, _, text)| text)
        .collect()
}

/// How well `text` matches `partial_lower`: `0` no match, `1` substring,
/// `2` a word starts with it, `3` the whole string starts with it (or the
/// partial is empty). Case-folded inputs.
fn match_rank(text_lower: &str, partial_lower: &str) -> u8 {
    if partial_lower.is_empty() {
        return 3;
    }
    if text_lower.starts_with(partial_lower) {
        return 3;
    }
    if text_lower
        .split_whitespace()
        .any(|word| word.starts_with(partial_lower))
    {
        return 2;
    }
    if text_lower.contains(partial_lower) {
        return 1;
    }
    0
}

/// Push `text` with its match rank when it matches and is not already
/// present (case-insensitively).
fn push_scored(scored: &mut Vec<(u8, usize, String)>, partial_lower: &str, text: &str) {
    let rank = match_rank(&text.to_ascii_lowercase(), partial_lower);
    if rank == 0 {
        return;
    }
    if scored.iter().any(|(_, _, e)| e.eq_ignore_ascii_case(text)) {
        return;
    }
    let order = scored.len();
    scored.push((rank, order, text.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers() -> Vec<String> {
        vec!["country".into(), "amount".into(), "status".into()]
    }

    fn ctx(input: &str, caret: usize) -> SuggestContext {
        context(input, caret, &headers(), 3)
    }

    fn end(input: &str) -> SuggestContext {
        context(input, input.len(), &headers(), 3)
    }

    #[test]
    fn column_slot_offers_row_scope_pseudo_columns() {
        let items = candidates(&end(""), &headers(), 3, None);
        assert_eq!(&items[0..2], &["any".to_string(), "all".to_string()]);
        assert_eq!(candidates(&end("al"), &headers(), 3, None), vec!["all"]);
    }

    #[test]
    fn row_scope_pseudo_column_moves_to_the_operator_and_value_slots() {
        let c = end("any ");
        assert_eq!(c.slot, Slot::Operator { column: None });
        assert!(candidates(&c, &headers(), 3, None).contains(&"empty".to_string()));

        let c = end("any == ");
        assert_eq!(
            c.slot,
            Slot::Value {
                column: None,
                op: FilterOp::Eq
            }
        );
        let values = vec!["US".to_string()];
        assert!(candidates(&c, &headers(), 3, Some(&values)).contains(&"US".to_string()));
    }

    #[test]
    fn empty_input_offers_columns() {
        let c = end("");
        assert_eq!(c.slot, Slot::Column);
        assert_eq!(c.partial, "");
        assert_eq!(c.replace, 0..0);
    }

    #[test]
    fn partial_column_offers_columns_with_prefix() {
        let c = end("cou");
        assert_eq!(c.slot, Slot::Column);
        assert_eq!(c.partial, "cou");
        assert_eq!(c.replace, 0..3);
        let items = candidates(&c, &headers(), 3, None);
        assert_eq!(items, vec!["country"]);
    }

    #[test]
    fn space_after_column_offers_operators() {
        let c = end("country ");
        assert_eq!(c.slot, Slot::Operator { column: Some(0) });
        assert_eq!(c.partial, "");
        let items = candidates(&c, &headers(), 3, None);
        assert!(items.contains(&"==".to_string()));
        assert!(items.contains(&"contains".to_string()));
    }

    #[test]
    fn partial_operator_is_filtered() {
        let c = end("country ==");
        assert_eq!(c.slot, Slot::Operator { column: Some(0) });
        assert_eq!(c.partial, "==");
        assert_eq!(candidates(&c, &headers(), 3, None), vec!["=="]);
    }

    #[test]
    fn space_after_operator_offers_values_for_the_column() {
        let c = end("country == ");
        assert_eq!(
            c.slot,
            Slot::Value {
                column: Some(0),
                op: FilterOp::Eq
            }
        );
        assert_eq!(c.partial, "");
        let values = vec!["US".to_string(), "UK".to_string()];
        let items = candidates(&c, &headers(), 3, Some(&values));
        assert!(items.contains(&"AND".to_string()));
        assert!(items.contains(&"US".to_string()));
        assert!(items.contains(&"UK".to_string()));
    }

    #[test]
    fn partial_value_is_filtered() {
        let c = end("country == U");
        assert_eq!(c.partial, "U");
        let values = vec!["US".to_string(), "UK".to_string(), "France".to_string()];
        assert_eq!(
            candidates(&c, &headers(), 3, Some(&values)),
            vec!["US", "UK"]
        );
    }

    #[test]
    fn value_less_operator_offers_keywords() {
        let c = end("notes empty ");
        assert_eq!(c.slot, Slot::Keyword);
        assert_eq!(candidates(&c, &headers(), 3, None), vec!["AND", "OR"]);
    }

    #[test]
    fn keyword_then_space_offers_columns_again() {
        let c = end("country == US AND ");
        assert_eq!(c.slot, Slot::Column);
        assert_eq!(c.partial, "");
    }

    #[test]
    fn open_paren_offers_columns() {
        let c = end("country == US AND (");
        assert_eq!(c.slot, Slot::Column);
    }

    #[test]
    fn boolean_keyword_ends_a_multi_word_value() {
        let c = end("city == New York AND ");
        assert_eq!(c.slot, Slot::Column);
        // And while the value is still being typed, we are still in Value.
        let mid = end("city == New York");
        assert_eq!(
            mid.slot,
            Slot::Value {
                column: None,
                op: FilterOp::Eq
            }
        );
        assert_eq!(mid.partial, "York");
    }

    #[test]
    fn caret_inside_a_word_uses_the_whole_word_as_the_replace_range() {
        // "country == US" with the caret between "U" and "S".
        let c = ctx("country == US", 12);
        assert_eq!(c.partial, "U");
        assert_eq!(c.replace, 11..13);
    }

    #[test]
    fn unknown_column_still_reports_an_operator_slot() {
        let c = end("nope ");
        assert_eq!(c.slot, Slot::Operator { column: None });
    }

    #[test]
    fn multibyte_partial_range_is_byte_correct() {
        let headers = vec!["ville".into()];
        let c = context("vil", 3, &headers, 1);
        assert_eq!(c.partial, "vil");
        assert_eq!(c.replace, 0..3);
        // "é" is two bytes; the range must stay on a char boundary.
        let c = context("é", 2, &headers, 1);
        assert_eq!(c.partial, "é");
        assert_eq!(c.replace, 0..2);
    }

    #[test]
    fn quoted_value_offers_without_quotes() {
        let c = end("country == \"U");
        assert_eq!(c.partial, "U");
        assert_eq!(
            c.slot,
            Slot::Value {
                column: Some(0),
                op: FilterOp::Eq
            }
        );
    }

    #[test]
    fn generated_col_label_resolves_to_a_value_slot() {
        // Headerless: the autocomplete inserts `colN`; it must resolve so
        // value suggestions populate.
        let c = context("col0 == ", 8, &[], 2);
        assert_eq!(
            c.slot,
            Slot::Value {
                column: Some(0),
                op: FilterOp::Eq
            }
        );
    }

    #[test]
    fn numeric_named_header_resolves_to_a_value_slot() {
        let headers = vec!["42".to_string(), "name".to_string()];
        let c = context("42 == ", 6, &headers, 2);
        assert_eq!(
            c.slot,
            Slot::Value {
                column: Some(0),
                op: FilterOp::Eq
            }
        );
    }

    #[test]
    fn quoted_multi_word_column_resolves_to_a_value_slot() {
        let headers = vec!["New York".to_string(), "pad".to_string()];
        let input = "\"New York\" == ";
        let c = context(input, input.len(), &headers, 2);
        assert_eq!(
            c.slot,
            Slot::Value {
                column: Some(0),
                op: FilterOp::Eq
            }
        );
    }

    #[test]
    fn value_matching_accepts_word_prefix_and_substring() {
        let values = vec!["New York".to_string(), "Boston".to_string()];
        // Typing the second word of a multi-word value still matches.
        assert_eq!(
            candidates(&end("city == Y"), &headers(), 3, Some(&values)),
            vec!["New York"]
        );
        // A mid-string substring matches too, ranked after prefixes.
        assert_eq!(
            candidates(&end("city == ost"), &headers(), 3, Some(&values)),
            vec!["Boston"]
        );
    }

    #[test]
    fn prefix_matches_rank_before_substring_matches() {
        let values = vec!["Bolus".to_string(), "US".to_string(), "Us-east".to_string()];
        assert_eq!(
            candidates(&end("k == us"), &headers(), 3, Some(&values)),
            vec!["US", "Us-east", "Bolus"]
        );
    }
}

//! Delimiter (and quote) detection.
//!
//! We sample the start of the file, split it into *logical* lines (respecting
//! quoted fields so embedded newlines do not break a record), then score each
//! candidate delimiter. The primary signal is the number of columns the
//! delimiter produces; consistency across sampled lines is the tie-breaker.
//!
//! This is good enough for the usual suspects: comma, tab, semicolon, pipe.
//! We never override a delimiter the user explicitly configured.

use std::borrow::Cow;

use crate::parser::{
    exact_string, strip_utf8_bom, Cells, HeaderMode, ParseMeta, ParseWarning, WarningKind,
};

/// Candidate delimiters we try. Order is the final tie-break (comma first,
/// because it is by far the most common data format).
pub const CANDIDATES: &[(u8, &str)] = &[(b',', ","), (b'\t', "\\t"), (b';', ";"), (b'|', "|")];

const SAMPLE_BYTES: usize = 64 * 1024;

/// How many following logical lines the header heuristic compares the
/// first line against. Bounded so a huge file is still cheap to sniff.
const HEADER_CONTEXT_ROWS: usize = 20;

#[derive(Debug)]
pub struct DetectOutcome {
    pub delimiter: u8,
    pub quote: u8,
    pub has_headers: bool,
    /// If `has_headers`, this is the decoded header list.
    pub headers: Vec<String>,
    pub warnings: Vec<ParseWarning>,
}

impl DetectOutcome {
    pub fn into_meta(self) -> ParseMeta {
        ParseMeta {
            delimiter: self.delimiter,
            quote: self.quote,
            has_headers: self.has_headers,
            column_count: self.headers.len(),
            headers: self.headers,
            record_count: 0,
            header_range: None,
            warnings: self.warnings,
        }
    }
}

/// Sniff delimiter + quote + headers from a byte sample, using the
/// automatic header heuristic.
pub fn detect(bytes: &[u8]) -> DetectOutcome {
    detect_with(bytes, HeaderMode::Auto)
}

/// Sniff delimiter + quote + headers from a byte sample. `mode` forces
/// the header interpretation (`Header` / `Headerless`) or runs the
/// content heuristic (`Auto`).
pub fn detect_with(bytes: &[u8], mode: HeaderMode) -> DetectOutcome {
    // A leading BOM is structural, not part of the first header/cell.
    // The bytes stay in the source; the encoding scan still reports the
    // BOM. See ADR 0003.
    let bytes = strip_utf8_bom(bytes);
    let sample = &bytes[..bytes.len().min(SAMPLE_BYTES)];
    let lines = split_logical_lines(sample, b'"');

    if lines.is_empty() {
        return DetectOutcome {
            delimiter: b',',
            quote: b'"',
            has_headers: false,
            headers: Vec::new(),
            warnings: vec![ParseWarning {
                kind: WarningKind::InconsistentColumnCount,
                record_index: None,
                note: "no data lines in sample".into(),
            }],
        };
    }

    let (delim, _columns) = best_delimiter(&lines);
    let first_line = lines[0];
    let first_cells: Vec<Cow<[u8]>> = Cells::new(first_line, delim, b'"').collect();

    let likely_header = match mode {
        // A forced header still requires at least one cell; an empty
        // first line stays headerless.
        HeaderMode::Header => !first_cells.is_empty(),
        HeaderMode::Headerless => false,
        HeaderMode::Auto => looks_like_header_in_context(&first_cells, &lines[1..], delim),
    };
    let headers = if likely_header {
        // Byte-faithful like every other displayed value: a header that
        // is not valid UTF-8 is escaped rather than replaced with U+FFFD.
        first_cells
            .iter()
            .map(|c| exact_string(c.as_ref()).into_owned())
            .collect()
    } else {
        Vec::new()
    };

    let mut warnings = Vec::new();
    if !headers.is_empty() {
        let mut seen: Vec<&str> = Vec::with_capacity(headers.len());
        for h in &headers {
            if h.trim().is_empty() {
                warnings.push(ParseWarning {
                    kind: WarningKind::EmptyHeader,
                    record_index: None,
                    note: "header has empty value".into(),
                });
            } else if seen.contains(&h.as_str()) {
                warnings.push(ParseWarning {
                    kind: WarningKind::DuplicateHeader,
                    record_index: None,
                    note: format!("duplicate header: {h:?}"),
                });
            }
            seen.push(h.as_str());
        }
    }

    DetectOutcome {
        delimiter: delim,
        quote: b'"',
        has_headers: !headers.is_empty(),
        headers,
        warnings,
    }
}

/// Splits the sample into logical lines, honouring quoted fields so a
/// newline inside quotes is not treated as a record boundary.
pub fn split_logical_lines(sample: &[u8], quote: u8) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut in_quotes = false;

    while i < sample.len() {
        let b = sample[i];
        if b == quote {
            if in_quotes && i + 1 < sample.len() && sample[i + 1] == quote {
                i += 2;
                continue;
            }
            in_quotes = !in_quotes;
            i += 1;
            continue;
        }
        if b == b'\n' && !in_quotes {
            let mut line = &sample[start..i];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if !line.is_empty() {
                lines.push(line);
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < sample.len() {
        let mut line = &sample[start..];
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

/// Picks the delimiter that produces the most columns in the first logical
/// line, breaking ties on cross-line consistency and then on candidate order.
fn best_delimiter(lines: &[&[u8]]) -> (u8, usize) {
    let mut best: Option<((usize, usize), u8, usize)> = None;
    for &(delim, _) in CANDIDATES {
        let counts: Vec<usize> = lines
            .iter()
            .map(|line| Cells::new(line, delim, b'"').count())
            .collect();
        let first = counts[0];
        if first == 0 {
            continue;
        }
        let mode = mode_of(&counts);
        let columns = first.max(mode);
        let matching = counts.iter().filter(|&&c| c == columns).count();
        let consistency = (matching * 1000) / counts.len();
        let score = (columns, consistency);

        let better = match &best {
            None => true,
            Some((best_score, _, _)) => score > *best_score,
        };
        if better {
            best = Some((score, delim, columns));
        }
    }
    best.map(|(_, d, c)| (d, c)).unwrap_or((b',', 1))
}

/// Most frequent value, breaking ties toward the larger count.
fn mode_of(counts: &[usize]) -> usize {
    let mut best = counts[0];
    let mut best_freq = 0usize;
    for &candidate in counts {
        let freq = counts.iter().filter(|&&c| c == candidate).count();
        if freq > best_freq || (freq == best_freq && candidate > best) {
            best = candidate;
            best_freq = freq;
        }
    }
    best
}

/// How a single cell reads: empty, numeric/boolean, or free text.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Textness {
    Empty,
    Typed,
    Text,
}

fn textness(bytes: &[u8]) -> Textness {
    let s = String::from_utf8_lossy(bytes);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Textness::Empty
    } else if looks_numeric(trimmed) || looks_boolean(trimmed) {
        Textness::Typed
    } else {
        Textness::Text
    }
}

/// Decides whether the first line is a header by comparing its cells
/// against the following data lines.
///
/// The signal is per column: a *header vote* is a text cell above a
/// typed (numeric/boolean) value (the classic `name` over `42` shape),
/// and a *data vote* is a typed cell above a typed value (the first line
/// looks like just another data row). Text-over-text is deliberately not
/// a data vote: `first_name,last_name` over `alice,bob` must keep
/// defaulting to a header. When the votes tie (or there is nothing to
/// compare), fall back to the local [`looks_like_header`] rule.
fn looks_like_header_in_context(
    first_cells: &[Cow<[u8]>],
    rest_lines: &[&[u8]],
    delimiter: u8,
) -> bool {
    if first_cells.len() < 2 {
        return false;
    }
    let mut header_votes = 0usize;
    let mut data_votes = 0usize;
    for line in rest_lines.iter().take(HEADER_CONTEXT_ROWS) {
        let cells: Vec<Cow<[u8]>> = Cells::new(line, delimiter, b'"').collect();
        for column in 0..first_cells.len().min(cells.len()) {
            let f = textness(first_cells[column].as_ref());
            let r = textness(cells[column].as_ref());
            if f == Textness::Empty || r == Textness::Empty {
                continue;
            }
            if f == Textness::Text && r == Textness::Typed {
                header_votes += 1;
            } else if f == Textness::Typed && r == Textness::Typed {
                data_votes += 1;
            }
        }
    }
    if header_votes > data_votes {
        true
    } else if data_votes > header_votes {
        false
    } else {
        looks_like_header(first_cells)
    }
}

/// Decides whether the first line is a header. A line is treated as data
/// (not a header) when at least half of its non-empty cells parse as
/// numbers or booleans. This is the local fallback used when comparing
/// the first line against the following data lines yields no signal.
fn looks_like_header(first_cells: &[Cow<[u8]>]) -> bool {
    if first_cells.len() < 2 {
        return false;
    }
    let sample_n = first_cells.len().min(8);
    let mut numeric_or_bool = 0usize;
    let mut considered = 0usize;
    for cell in first_cells.iter().take(sample_n) {
        let s = String::from_utf8_lossy(cell.as_ref());
        let trimmed = s.trim();
        if trimmed.is_empty() {
            continue;
        }
        considered += 1;
        if looks_numeric(trimmed) || looks_boolean(trimmed) {
            numeric_or_bool += 1;
        }
    }
    considered >= 2 && numeric_or_bool * 2 < considered
}

fn looks_numeric(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars().peekable();
    if matches!(chars.peek(), Some('-' | '+')) {
        chars.next();
    }
    let mut saw_dot = false;
    let mut saw_digit = false;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            saw_digit = true;
            chars.next();
        } else if c == '.' && !saw_dot {
            saw_dot = true;
            chars.next();
        } else {
            return false;
        }
    }
    saw_digit
}

fn looks_boolean(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "true" | "false" | "yes" | "no" | "y" | "n" | "t" | "f"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_header_over_typed_data_is_a_header() {
        let out = detect(b"name,age\nalice,30\nbob,40\n");
        assert!(out.has_headers);
        assert_eq!(out.headers, vec!["name", "age"]);
    }

    #[test]
    fn typed_first_row_over_typed_data_is_headerless() {
        let out = detect(b"1,2\n3,4\n5,6\n");
        assert!(!out.has_headers);
    }

    #[test]
    fn numeric_first_row_over_text_data_is_headerless() {
        let out = detect(b"1,2,3\na,b,c\n");
        assert!(!out.has_headers);
    }

    /// The genuinely ambiguous all-text case stays a header under Auto.
    #[test]
    fn all_text_over_all_text_defaults_to_header() {
        let out = detect(b"first_name,last_name\nalice,bob\n");
        assert!(out.has_headers);
    }

    /// A headerless file whose first row matches the data shape is now
    /// detected even though the first row contains text.
    #[test]
    fn name_over_age_shape_is_headerless() {
        let out = detect(b"alice,30\ncarol,40\n");
        assert!(!out.has_headers);
    }

    #[test]
    fn force_header_mode_treats_numeric_first_row_as_header() {
        let out = detect_with(b"1,2\n3,4\n", HeaderMode::Header);
        assert!(out.has_headers);
        assert_eq!(out.headers, vec!["1", "2"]);
    }

    #[test]
    fn force_headerless_mode_ignores_a_text_header() {
        let out = detect_with(b"name,age\nalice,30\n", HeaderMode::Headerless);
        assert!(!out.has_headers);
        assert!(out.headers.is_empty());
    }

    #[test]
    fn auto_detect_matches_detect_with_auto() {
        let bytes = b"a,b\n1,2\n";
        let plain = detect(bytes);
        let explicit = detect_with(bytes, HeaderMode::Auto);
        assert_eq!(plain.has_headers, explicit.has_headers);
        assert_eq!(plain.headers, explicit.headers);
    }
}

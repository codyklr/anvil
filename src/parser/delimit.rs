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

use crate::parser::{Cells, ParseMeta, ParseWarning, WarningKind};

/// Candidate delimiters we try. Order is the final tie-break (comma first,
/// because it is by far the most common data format).
pub const CANDIDATES: &[(u8, &str)] = &[(b',', ","), (b'\t', "\\t"), (b';', ";"), (b'|', "|")];

const SAMPLE_BYTES: usize = 64 * 1024;

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

/// Sniff delimiter + quote + headers from a byte sample.
pub fn detect(bytes: &[u8]) -> DetectOutcome {
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

    let likely_header = looks_like_header(&first_cells);
    let headers = if likely_header {
        first_cells
            .iter()
            .map(|c| String::from_utf8_lossy(c.as_ref()).into_owned())
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

/// Decides whether the first line is a header. A line is treated as data
/// (not a header) when at least half of its non-empty cells parse as
/// numbers or booleans.
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

//! CSV/TSV/delimited-text parsing.
//!
//! Strategy for Anvil:
//!  - We do NOT keep parsed rows in memory as Vec<Vec<String>>.
//!  - We index the file by recording start/end byte offsets of each record.
//!  - When the grid needs a row, it asks the index for the byte range and
//!    decodes the relevant columns on the fly using a state machine that
//!    walks only that record's bytes.
//!  - We pre-decode the header row because every render needs it.

pub mod delimit;
pub mod indexing;

use std::borrow::Cow;

use crate::source::OpenedSource;

/// Strongly typed column index. Prevents accidental arithmetic with row
/// indices and capacities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColumnIndex(pub usize);

/// Persisted layout decisions made about a file.
#[derive(Debug, Clone)]
pub struct ParseMeta {
    pub delimiter: u8,
    pub quote: u8,
    pub has_headers: bool,
    /// Headers decoded from the first logical line (if `has_headers`).
    pub headers: Vec<String>,
    /// Total record count (excluding header row, if any).
    pub record_count: u64,
    /// Number of columns. Derived from the headers when present, otherwise
    /// from the widest data record seen while indexing. Always correct even
    /// for headerless files.
    pub column_count: usize,
    /// Header line byte range (start inclusive, end exclusive). Useful
    /// when later diagnostics need to highlight the header.
    pub header_range: Option<(u64, u64)>,
    /// Diagnostic warnings collected during indexing.
    pub warnings: Vec<ParseWarning>,
}

/// Display name for a column: the header when present and non-empty,
/// otherwise a zero-based generated label (`col0`, `col1`, …).
/// Headerless files leave `headers` empty, so they get generated labels
/// while `has_headers` stays meaningful.
pub fn column_label(headers: &[String], column: usize) -> String {
    headers
        .get(column)
        .filter(|h| !h.is_empty())
        .cloned()
        .unwrap_or_else(|| format!("col{column}"))
}

impl ParseMeta {
    /// Display name for `column` (see [`column_label`]).
    pub fn column_label(&self, column: usize) -> String {
        column_label(&self.headers, column)
    }
}

/// How the first logical line is interpreted.
///
/// `Auto` runs the content heuristic; `Header` and `Headerless` are the
/// explicit user overrides that skip the heuristic entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderMode {
    #[default]
    Auto,
    /// Always treat the first row as the header (even all-numeric data).
    Header,
    /// Never treat the first row as a header; every row is data.
    Headerless,
}

#[derive(Debug, Clone)]
pub struct ParseWarning {
    pub kind: WarningKind,
    pub record_index: Option<u64>,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    InconsistentColumnCount,
    EmptyHeader,
    DuplicateHeader,
    UnterminatedQuote,
    VeryLongRecord,
}

impl std::fmt::Display for WarningKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InconsistentColumnCount => write!(f, "inconsistent column count"),
            Self::EmptyHeader => write!(f, "empty header"),
            Self::DuplicateHeader => write!(f, "duplicate header"),
            Self::UnterminatedQuote => write!(f, "unterminated quote"),
            Self::VeryLongRecord => write!(f, "very long record"),
        }
    }
}

/// A document indexed from an `OpenedSource`. Cheap to read because it
/// only carries an Arc to the mmap and Vec<u64> of record start offsets.
pub struct ParsedDocument {
    pub opened: OpenedSource,
    pub meta: ParseMeta,
    /// Start byte offset of each record (data row), in ascending order.
    /// The end of record `i` is `record_offsets[i+1]`. The last entry
    /// equals the total length of the source. So `len()` is
    /// `record_count + 1` when every record is followed by a newline,
    /// and we always have a sentinel to make iteration uniform.
    pub record_offsets: Vec<u64>,
}

impl ParsedDocument {
    pub fn record_count(&self) -> u64 {
        self.meta.record_count
    }

    pub fn column_count(&self) -> usize {
        self.meta.column_count
    }

    /// Returns the byte range (start inclusive, end exclusive) of record
    /// at `record_index`. Returns `None` if `record_index` is out of range.
    pub fn record_range(&self, record_index: u64) -> Option<(u64, u64)> {
        let i = record_index as usize;
        if i + 1 >= self.record_offsets.len() {
            return None;
        }
        Some((self.record_offsets[i], self.record_offsets[i + 1]))
    }

    /// Returns the bytes of a record (no trailing newline).
    pub fn record_bytes(&self, record_index: u64) -> Option<&[u8]> {
        let (start, end) = self.record_range(record_index)?;
        let bytes = self.opened.bytes();
        let mut hi = end as usize;
        // trim a single trailing newline if present; we don't trim \r\n
        // because CSV uses bare \n; .tsv / CSV may still have \r\n. Strip
        // a final \r before that.
        if hi > start as usize && bytes[hi - 1] == b'\n' {
            hi -= 1;
            if hi > start as usize && bytes[hi - 1] == b'\r' {
                hi -= 1;
            }
        }
        Some(&bytes[start as usize..hi])
    }

    /// Decodes a single cell value lazily by slicing the underlying mmap.
    pub fn cell(&self, record_index: u64, column: ColumnIndex) -> Option<Cell<'_>> {
        let record = self.record_bytes(record_index)?;
        let raw = cell_at(record, column.0, self.meta.delimiter, self.meta.quote)?;
        Some(Cell { inner: raw })
    }

    /// Convenience: decode an entire record into owned Strings.
    /// Used for diagnostics and search. The grid should call `cell`
    /// instead to keep allocations down.
    pub fn record_strings(&self, record_index: u64) -> Vec<String> {
        let Some(record) = self.record_bytes(record_index) else {
            return Vec::new();
        };
        Cells::new(record, self.meta.delimiter, self.meta.quote)
            .map(|c| Cell { inner: c }.into_string())
            .collect()
    }
}

/// A single cell value, as either a borrowed slice into the mmap or
/// a small owned allocation when the original cell contained escape
/// sequences that had to be rewritten.
#[derive(Debug, Clone)]
pub struct Cell<'a> {
    inner: Cow<'a, [u8]>,
}

impl<'a> Cell<'a> {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.inner).unwrap_or("")
    }

    /// Whether the cell's decoded bytes are valid UTF-8. `as_str` and
    /// `into_string` silently fall back to lossy/empty for invalid
    /// input, so this is the flag the encoding diagnostics scan uses to
    /// count cells that would otherwise be hidden.
    pub fn is_utf8(&self) -> bool {
        std::str::from_utf8(&self.inner).is_ok()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn into_string(self) -> String {
        match self.inner {
            Cow::Borrowed(b) => std::str::from_utf8(b).unwrap_or("").to_string(),
            Cow::Owned(v) => match String::from_utf8(v) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
            },
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.inner
    }

    /// The cell's bytes as exact, displayable text.
    ///
    /// Valid UTF-8 is returned verbatim. A cell that contains at least
    /// one byte sequence that is not valid UTF-8 is escaped rather than
    /// hidden or replaced, so the grid and the clipboard always show
    /// bytes that are really in the file. See [`exact_string`].
    pub fn to_exact_string(&self) -> Cow<'_, str> {
        exact_string(&self.inner)
    }
}

/// Render a cell's bytes as exact text without losing information.
///
/// When the bytes are valid UTF-8 the string is returned borrowed and
/// unchanged, so the common path costs nothing. Otherwise the value is
/// escaped: a literal backslash becomes `\\` and each invalid byte
/// becomes `\xNN` (lowercase hex). Escaping backslashes across the whole
/// cell - not just the invalid runs - is what makes the mapping
/// reversible, because a `\\` can never be confused with the start of
/// an escape. Nothing is ever replaced with U+FFFD and nothing is
/// dropped.
///
pub fn exact_string(bytes: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(valid) => Cow::Borrowed(valid),
        Err(_) => Cow::Owned(escape_invalid_utf8(bytes)),
    }
}

/// Byte-faithful escaped text for at most the first `max_bytes` of
/// `bytes`. Returns the text and whether any bytes were dropped.
///
/// Used by the full-cell viewer so a multi-MB cell cannot force the UI
/// to shape megabytes of text. The escaping rules match [`exact_string`]:
/// if the whole slice is valid UTF-8 the prefix is backed off to a
/// character boundary so the last shown character is intact; otherwise
/// the prefix is escaped with the same `\xNN` form (a cut inside an
/// invalid run is safe because those bytes are displayed as escapes).
/// The caller is responsible for reporting the truncation.
pub fn exact_string_capped(bytes: &[u8], max_bytes: usize) -> (Cow<'_, str>, bool) {
    if bytes.len() <= max_bytes {
        return (exact_string(bytes), false);
    }
    let prefix = &bytes[..max_bytes];
    if let Ok(valid) = std::str::from_utf8(prefix) {
        return (Cow::Borrowed(valid), true);
    }
    if std::str::from_utf8(bytes).is_ok() {
        // The whole slice is valid UTF-8, so the cut split a multi-byte
        // character. Back off to the boundary (at most 3 bytes).
        let mut end = max_bytes;
        while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
            end -= 1;
        }
        return (
            Cow::Borrowed(std::str::from_utf8(&bytes[..end]).unwrap_or_default()),
            true,
        );
    }
    (Cow::Owned(escape_invalid_utf8(prefix)), true)
}

/// Copy the valid parts of `text` into `out`, doubling literal
/// backslashes so escaped output stays unambiguous.
fn push_valid(out: &mut String, text: &str) {
    for ch in text.chars() {
        if ch == '\\' {
            out.push_str("\\\\");
        } else {
            out.push(ch);
        }
    }
}

/// Escape a byte slice that is not valid UTF-8: valid runs verbatim
/// (with backslashes doubled) and every invalid byte as `\xNN`.
fn escape_invalid_utf8(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len() + 8);
    let mut i = 0;
    while i < bytes.len() {
        match std::str::from_utf8(&bytes[i..]) {
            Ok(tail) => {
                push_valid(&mut out, tail);
                break;
            }
            Err(err) => {
                let valid = err.valid_up_to();
                if valid > 0 {
                    // The prefix is valid UTF-8 by construction.
                    let prefix = std::str::from_utf8(&bytes[i..i + valid]).unwrap_or_default();
                    push_valid(&mut out, prefix);
                    i += valid;
                }
                let bad = err.error_len().unwrap_or(bytes.len() - i);
                for byte in &bytes[i..i + bad] {
                    if *byte == b'\\' {
                        out.push_str("\\\\");
                    } else {
                        let _ = write!(out, "\\x{byte:02x}");
                    }
                }
                i += bad;
            }
        }
    }
    out
}

/// The UTF-8 byte-order mark, `EF BB BF`.
pub const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Length of a leading UTF-8 BOM in `bytes`, or `0`.
pub fn bom_len(bytes: &[u8]) -> usize {
    if bytes.starts_with(&UTF8_BOM) {
        UTF8_BOM.len()
    } else {
        0
    }
}

/// `bytes` without a leading UTF-8 BOM. The BOM bytes stay in the
/// underlying source (the encoding scan still reports them); only the
/// parsed view skips them.
pub fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    &bytes[bom_len(bytes)..]
}

/// Iterator over the cells of a single record.
pub struct Cells<'a> {
    record: &'a [u8],
    delimiter: u8,
    quote: u8,
    i: usize,
    /// Start byte of the cell we're about to emit.
    cell_start: usize,
    /// True when we've already returned the last cell of the record.
    done: bool,
}

impl<'a> Cells<'a> {
    pub fn new(record: &'a [u8], delimiter: u8, quote: u8) -> Self {
        Self {
            record,
            delimiter,
            quote,
            i: 0,
            cell_start: 0,
            done: false,
        }
    }
}

impl<'a> Iterator for Cells<'a> {
    type Item = Cow<'a, [u8]>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut in_quotes = false;
        if self.i == self.cell_start
            && self.i < self.record.len()
            && self.record[self.i] == self.quote
        {
            in_quotes = true;
            self.i += 1;
        }

        loop {
            if self.i >= self.record.len() {
                let cell_end = self.record.len();
                let start = self.cell_start;
                let raw = &self.record[start..cell_end];
                let decoded = unquote_cell(raw, self.quote);
                self.cell_start = cell_end + 1;
                self.done = true;
                return Some(decoded);
            }
            let b = self.record[self.i];
            if in_quotes {
                if b == self.quote {
                    if self.i + 1 < self.record.len() && self.record[self.i + 1] == self.quote {
                        // Escaped quote inside quoted field.
                        self.i += 2;
                        continue;
                    }
                    // End of quoted field.
                    in_quotes = false;
                    self.i += 1;
                    continue;
                }
                self.i += 1;
            } else {
                if b == self.delimiter {
                    let cell_end = self.i;
                    let start = self.cell_start;
                    let raw = &self.record[start..cell_end];
                    let decoded = unquote_cell(raw, self.quote);
                    self.i = cell_end + 1;
                    self.cell_start = self.i;
                    return Some(decoded);
                }
                if b == self.quote && self.i == self.cell_start {
                    in_quotes = true;
                    self.i += 1;
                    continue;
                }
                self.i += 1;
            }
        }
    }
}

/// Returns the bytes of cell `col` from `record`, or `None` if the record
/// has fewer columns. O(record length) per call.
pub fn cell_at<'a>(
    record: &'a [u8],
    col: usize,
    delimiter: u8,
    quote: u8,
) -> Option<Cow<'a, [u8]>> {
    Cells::new(record, delimiter, quote).nth(col)
}

/// If `cell` looks like a quoted field (starts and ends with quote),
/// strip the surrounding quotes and unescape doubled quotes. Otherwise
/// return the inputborrowed unchanged.
fn unquote_cell<'a>(cell: &'a [u8], quote: u8) -> Cow<'a, [u8]> {
    if cell.len() < 2 || cell[0] != quote || cell[cell.len() - 1] != quote {
        return Cow::Borrowed(cell);
    }
    let inner = &cell[1..cell.len() - 1];
    // Fast path: no internal quote → borrow.
    if !memchr::memchr(quote, inner).is_some() {
        return Cow::Borrowed(inner);
    }
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        if inner[i] == quote && i + 1 < inner.len() && inner[i + 1] == quote {
            out.push(quote);
            i += 2;
        } else {
            out.push(inner[i]);
            i += 1;
        }
    }
    Cow::Owned(out)
}

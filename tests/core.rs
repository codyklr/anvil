//! Integration tests for Anvil's data-side logic (no GPUI).
//!
//! These cover CSV/TSV parsing, quoting, indexing, diagnostics, and the
//! archive abstraction including ZIP safety checks.

use std::io::Write;
use std::path::PathBuf;

use anvil_core::source::local::open_local;
use anvil_core::{
    apply_filter, exact_string, infer_schema, open_path, parse_filter, parse_opened,
    parse_opened_with, reindex, scan_encoding, search_records, sort_rows, ArchiveSource, CellPos,
    Cells, ColumnIndex, HeaderMode, IndexOptions, LoadResult, OpenResult, OpenedSource,
    ParsedDocument, SearchOptions, Selection, SortKind, SortOrder, SortSpec,
};

// ---- helpers ------------------------------------------------------------

fn write_temp(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    (dir, path)
}

fn parse_bytes(bytes: &[u8]) -> (ParsedDocument, tempfile::TempDir) {
    let (dir, path) = write_temp("data.csv", bytes);
    let opened: OpenedSource = open_local(&path).expect("open local");
    let result = parse_opened(&opened);
    let doc = ParsedDocument {
        opened,
        meta: result.meta,
        record_offsets: result.record_offsets,
    };
    (doc, dir)
}

fn cell(doc: &ParsedDocument, row: u64, col: usize) -> String {
    doc.cell(row, ColumnIndex(col))
        .map(|c| c.into_string())
        .unwrap_or_default()
}

fn make_zip(name: &str, entries: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    let file = std::fs::File::create(&path).expect("create zip");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (entry_name, bytes) in entries {
        zip.start_file(*entry_name, opts).expect("start file");
        zip.write_all(bytes).expect("write entry");
    }
    zip.finish().expect("finish zip");
    (dir, path)
}

// ---- CSV parsing --------------------------------------------------------

#[test]
fn parses_normal_csv_with_headers() {
    let (doc, _d) = parse_bytes(b"id,name,score\n1,alice,10\n2,bob,20\n");
    assert_eq!(doc.column_count(), 3);
    assert_eq!(doc.meta.headers, vec!["id", "name", "score"]);
    assert_eq!(doc.record_count(), 2);
    assert_eq!(cell(&doc, 0, 0), "1");
    assert_eq!(cell(&doc, 0, 1), "alice");
    assert_eq!(cell(&doc, 1, 2), "20");
}

#[test]
fn parses_tsv_via_delimiter_detection() {
    let (doc, _d) = parse_bytes(b"id\tname\tscore\n1\talice\t10\n2\tbob\t20\n");
    assert_eq!(doc.meta.delimiter, b'\t');
    assert_eq!(doc.meta.headers, vec!["id", "name", "score"]);
    assert_eq!(cell(&doc, 1, 1), "bob");
}

#[test]
fn parses_semicolon_and_pipe() {
    for delim in *b";|" {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"a");
        bytes.push(delim);
        bytes.extend_from_slice(b"b");
        bytes.push(b'\n');
        bytes.extend_from_slice(b"1");
        bytes.push(delim);
        bytes.extend_from_slice(b"2");
        bytes.push(b'\n');
        let (doc, _d) = parse_bytes(&bytes);
        assert_eq!(doc.meta.delimiter, delim, "delimiter {delim}");
    }
}

#[test]
fn quoted_delimiter_is_not_a_separator() {
    let (doc, _d) = parse_bytes(b"a,b\n\"x,y\",z\n");
    assert_eq!(doc.record_count(), 1);
    assert_eq!(cell(&doc, 0, 0), "x,y");
    assert_eq!(cell(&doc, 0, 1), "z");
}

#[test]
fn quoted_newline_spans_records() {
    let (doc, _d) = parse_bytes(b"a,b\n\"line1\nline2\",z\nnext,q\n");
    assert_eq!(doc.record_count(), 2);
    assert_eq!(cell(&doc, 0, 0), "line1\nline2");
    assert_eq!(cell(&doc, 0, 1), "z");
    assert_eq!(cell(&doc, 1, 0), "next");
}

#[test]
fn escaped_quotes_are_unescaped() {
    let (doc, _d) = parse_bytes(b"a,b\n\"say \"\"hi\"\"\",z\n");
    assert_eq!(cell(&doc, 0, 0), "say \"hi\"");
    assert_eq!(cell(&doc, 0, 1), "z");
}

#[test]
fn missing_values_are_empty_cells() {
    let (doc, _d) = parse_bytes(b"a,b,c\n1,,3\n");
    assert_eq!(cell(&doc, 0, 0), "1");
    assert_eq!(cell(&doc, 0, 1), "");
    assert_eq!(cell(&doc, 0, 2), "3");
}

#[test]
fn trailing_delimiter_yields_empty_cell() {
    let (doc, _d) = parse_bytes(b"a,b,c\n1,2,\n");
    assert_eq!(doc.column_count(), 3);
    assert_eq!(cell(&doc, 0, 2), "");
}

#[test]
fn empty_column_values_rendered_empty() {
    let (doc, _d) = parse_bytes(b"a,b\n,1\n,2\n");
    assert_eq!(cell(&doc, 0, 0), "");
    assert_eq!(cell(&doc, 1, 0), "");
}

#[test]
fn no_trailing_newline_still_indexes_last_record() {
    let (doc, _d) = parse_bytes(b"a,b\n1,2\n3,4");
    assert_eq!(doc.record_count(), 2);
    assert_eq!(cell(&doc, 1, 1), "4");
}

#[test]
fn headerless_no_trailing_newline_counts_last_record() {
    let (doc, _d) = parse_bytes(b"1,2,3\n4,5,6");
    assert!(!doc.meta.has_headers);
    assert_eq!(doc.record_count(), 2);
    assert_eq!(doc.column_count(), 3);
    assert_eq!(cell(&doc, 1, 2), "6");
}

#[test]
fn single_value_file_without_newline() {
    let (doc, _d) = parse_bytes(b"hello");
    assert_eq!(doc.record_count(), 1);
    assert_eq!(doc.column_count(), 1);
    assert_eq!(cell(&doc, 0, 0), "hello");
}

#[test]
fn crlf_newlines_are_handled() {
    let (doc, _d) = parse_bytes(b"a,b\r\n1,2\r\n3,4\r\n");
    assert_eq!(doc.record_count(), 2);
    assert_eq!(cell(&doc, 0, 1), "2");
}

// ---- headers / diagnostics ---------------------------------------------

#[test]
fn duplicate_headers_warn() {
    let (doc, _d) = parse_bytes(b"a,a,b\n1,2,3\n");
    let has_dup = doc
        .meta
        .warnings
        .iter()
        .any(|w| w.kind == anvil_core::WarningKind::DuplicateHeader);
    assert!(has_dup, "expected duplicate-header warning");
}

#[test]
fn empty_header_warns() {
    let (doc, _d) = parse_bytes(b"a,,c\n1,2,3\n");
    let has_empty = doc
        .meta
        .warnings
        .iter()
        .any(|w| w.kind == anvil_core::WarningKind::EmptyHeader);
    assert!(has_empty, "expected empty-header warning");
}

#[test]
fn inconsistent_row_length_warns() {
    let (doc, _d) = parse_bytes(b"a,b,c\n1,2,3\n4,5\n");
    let has_inconsistent = doc
        .meta
        .warnings
        .iter()
        .any(|w| w.kind == anvil_core::WarningKind::InconsistentColumnCount);
    assert!(has_inconsistent, "expected inconsistent-row warning");
    assert_eq!(doc.record_count(), 2);
}

#[test]
fn file_without_headers_is_detected() {
    let (doc, _d) = parse_bytes(b"1,2,3\n4,5,6\n7,8,9\n");
    assert!(!doc.meta.has_headers);
    assert!(doc.meta.headers.is_empty());
    assert_eq!(doc.record_count(), 3);
    assert_eq!(cell(&doc, 0, 0), "1");
    // The grid needs a column count even without headers.
    assert_eq!(doc.column_count(), 3);
}

#[test]
fn headerless_text_rows_are_detected_when_the_shape_matches_data() {
    // `alice,30` is text-over-typed just like the following rows, so the
    // first row is data, not a header.
    let (doc, _d) = parse_bytes(b"alice,30\ncarol,40\n");
    assert!(!doc.meta.has_headers);
    assert_eq!(doc.record_count(), 2);
    assert_eq!(cell(&doc, 0, 0), "alice");
    // Headerless columns get generated `colN` labels.
    assert_eq!(doc.meta.column_label(0), "col0");
    assert_eq!(doc.meta.column_label(1), "col1");
}

#[test]
fn forced_header_mode_overrides_detection() {
    let (_dir, path) = write_temp("headers.csv", b"alice,bob\ncarol,dave\n");
    let opened = open_local(&path).expect("open");
    // All-text is ambiguous and defaults to a header under Auto.
    let auto = parse_opened(&opened);
    assert!(auto.meta.has_headers);
    assert_eq!(auto.meta.record_count, 1);

    let headerless = parse_opened_with(
        &opened,
        IndexOptions {
            header_mode: HeaderMode::Headerless,
            ..IndexOptions::default()
        },
    );
    assert!(!headerless.meta.has_headers);
    assert_eq!(headerless.meta.record_count, 2);
    assert_eq!(headerless.meta.column_count, 2);
}

#[test]
fn reindex_reuses_the_opened_source() {
    let (_dir, path) = write_temp("reindex.csv", b"a,b\n1,2\n3,4\n");
    let opened = open_local(&path).expect("open");
    let doc = anvil_core::index_opened(opened);
    assert!(doc.meta.has_headers);

    let forced = reindex(
        &doc,
        IndexOptions {
            header_mode: HeaderMode::Headerless,
            ..IndexOptions::default()
        },
    );
    assert!(!forced.meta.has_headers);
    assert_eq!(forced.meta.record_count, 3);

    // Dropping the original must not invalidate the clone's mmap.
    drop(doc);
    assert_eq!(cell(&forced, 2, 1), "4");
}

// ---- scale / pathological inputs ---------------------------------------

#[test]
fn extremely_wide_file() {
    let cols = 2000;
    let mut header = String::new();
    let mut row = String::new();
    for i in 0..cols {
        if i > 0 {
            header.push(',');
            row.push(',');
        }
        header.push_str(&format!("c{i}"));
        row.push_str(&format!("{i}"));
    }
    let bytes = format!("{header}\n{row}\n");
    let (doc, _d) = parse_bytes(bytes.as_bytes());
    assert_eq!(doc.column_count(), cols);
    assert_eq!(cell(&doc, 0, 1999), "1999");
}

#[test]
fn extremely_long_cell() {
    let big = "x".repeat(1_000_000);
    let bytes = format!("a,b\n\"{big}\",z\n");
    let (doc, _d) = parse_bytes(bytes.as_bytes());
    assert_eq!(cell(&doc, 0, 0).len(), 1_000_000);
    assert_eq!(cell(&doc, 0, 1), "z");
}

#[test]
fn very_long_record_warns() {
    // 2 MiB of cell content exceeds the 1 MiB record-length diagnostic.
    let big = "x".repeat(2 * 1024 * 1024);
    let bytes = format!("a,b\nsmall,1\n{big},2\n");
    let (doc, _d) = parse_bytes(bytes.as_bytes());
    let warning = doc
        .meta
        .warnings
        .iter()
        .find(|w| w.kind == anvil_core::WarningKind::VeryLongRecord)
        .expect("very-long-record warning");
    assert_eq!(warning.record_index, Some(1));
    assert_eq!(doc.record_count(), 2);
    assert_eq!(cell(&doc, 0, 0), "small");
}

#[test]
fn ordinary_records_do_not_warn_as_very_long() {
    // A 1M-character cell stays just under the 1 MiB limit and must not
    // produce a false positive.
    let big = "x".repeat(1_000_000);
    let bytes = format!("a,b\n\"{big}\",z\n");
    let (doc, _d) = parse_bytes(bytes.as_bytes());
    assert!(doc
        .meta
        .warnings
        .iter()
        .all(|w| w.kind != anvil_core::WarningKind::VeryLongRecord));
}

#[test]
fn very_long_record_threshold_is_configurable() {
    let (_dir, path) = write_temp("threshold.csv", b"a,b\n1,2\n3,4\n");
    let opened = open_local(&path).expect("open");
    // Each data record ("1,2\n") is 4 bytes including its newline.
    let lenient = parse_opened_with(
        &opened,
        IndexOptions {
            max_record_bytes: 4,
            ..IndexOptions::default()
        },
    );
    assert!(lenient
        .meta
        .warnings
        .iter()
        .all(|w| w.kind != anvil_core::WarningKind::VeryLongRecord));

    let strict = parse_opened_with(
        &opened,
        IndexOptions {
            max_record_bytes: 3,
            ..IndexOptions::default()
        },
    );
    let flagged: Vec<u64> = strict
        .meta
        .warnings
        .iter()
        .filter(|w| w.kind == anvil_core::WarningKind::VeryLongRecord)
        .filter_map(|w| w.record_index)
        .collect();
    assert_eq!(flagged, vec![0, 1]);
}

#[test]
fn cell_is_utf8_reports_invalid_bytes() {
    let (doc, _d) = parse_bytes(b"a,b\n\xff,ok\n");
    assert!(!doc.cell(0, ColumnIndex(0)).expect("cell").is_utf8());
    assert!(doc.cell(0, ColumnIndex(1)).expect("cell").is_utf8());
}

#[test]
fn encoding_scan_flags_invalid_column() {
    let (doc, _d) = parse_bytes(b"a,b\n1,\xff\n2,\xfe\n");
    let report = scan_encoding(&doc);
    assert!(report.has_invalid_utf8());
    assert_eq!(report.total_invalid_cells(), 2);
    assert_eq!(report.columns.len(), 1);
    assert_eq!(report.columns[0].column, 1);
    assert_eq!(report.columns[0].name, "b");
    assert_eq!(report.columns[0].invalid_cells, 2);
}

/// Data fidelity: bytes that are not valid UTF-8 must stay visible. The
/// old behaviour rendered such a cell as if it were empty, which hid the
/// value entirely.
#[test]
fn invalid_utf8_cells_are_shown_exactly() {
    let (doc, _d) = parse_bytes(b"a,b\n1,\xff\n2,x\\y\xfe\n");

    // Record 0 is the first data row; the header is not a record.
    let first = doc.cell(0, ColumnIndex(1)).expect("cell");
    assert!(!first.is_utf8());
    assert_eq!(first.to_exact_string(), "\\xff");

    // A literal backslash inside a cell that also holds an invalid byte
    // is doubled, so the escaped form stays reversible.
    let second = doc.cell(1, ColumnIndex(1)).expect("cell");
    assert_eq!(second.to_exact_string(), "x\\\\y\\xfe");

    // A cell of valid UTF-8 is returned unchanged, backslash included.
    let plain = doc.cell(0, ColumnIndex(0)).expect("cell");
    assert!(plain.is_utf8());
    assert_eq!(plain.to_exact_string(), "1");
}

#[test]
fn exact_string_escapes_only_when_it_has_to() {
    assert_eq!(exact_string(b"plain"), "plain");
    // Valid UTF-8 keeps a real backslash exactly as written.
    assert_eq!(exact_string(b"a\\b"), r"a\b");
    assert_eq!(exact_string(b"\xff"), r"\xff");
    assert_eq!(exact_string(b"ok\x80\x81"), r"ok\x80\x81");
    // Invalid cell: every backslash is doubled to keep the escapes
    // unambiguous.
    assert_eq!(exact_string(b"a\\b\xff"), r"a\\b\xff");
    // A truncated multi-byte sequence at the end of the cell.
    assert_eq!(exact_string(b"\xe2\x82"), r"\xe2\x82");
}

/// A header that is not valid UTF-8 is part of the data too, so it must
/// reach the UI escaped rather than as U+FFFD.
#[test]
fn invalid_utf8_headers_are_escaped_not_replaced() {
    let (doc, _d) = parse_bytes(b"a,\xffh\n1,2\n");
    assert_eq!(
        doc.meta.headers,
        vec!["a".to_string(), "\\xffh".to_string()]
    );
}

#[test]
fn large_synthetic_dataset_indexes_completely() {
    let rows = 100_000;
    let mut bytes = String::with_capacity(rows * 24);
    bytes.push_str("id,name,value\n");
    for i in 0..rows {
        bytes.push_str(&format!("{i},name{i},{}\n", i * 3));
    }
    let (doc, _d) = parse_bytes(bytes.as_bytes());
    assert_eq!(doc.record_count(), rows as u64);
    assert_eq!(cell(&doc, rows as u64 - 1, 0), (rows - 1).to_string());
    assert_eq!(cell(&doc, rows as u64 - 1, 2), ((rows - 1) * 3).to_string());
}

#[test]
fn record_count_matches_record_offsets_length() {
    let (doc, _d) = parse_bytes(b"a,b\n1,2\n3,4\n5,6\n");
    assert_eq!(doc.record_offsets.len() as u64, doc.record_count() + 1);
}

// ---- regression: quoting in later columns ---------------------------------

#[test]
fn quoted_comma_in_later_column_does_not_add_columns() {
    // A quote is only special at the start of a field, but it must be
    // recognised there for *every* column, not just the first.
    let (doc, _d) = parse_bytes(b"a,b,c\n1,\"x,y\",3\n4,\"p,q\",6\n");
    assert_eq!(doc.column_count(), 3);
    assert_eq!(doc.record_count(), 2);
    assert!(
        !doc.meta
            .warnings
            .iter()
            .any(|w| w.kind == anvil_core::WarningKind::InconsistentColumnCount),
        "well-formed quoted commas must not warn: {:?}",
        doc.meta.warnings
    );
    assert_eq!(cell(&doc, 0, 1), "x,y");
    assert_eq!(cell(&doc, 1, 1), "p,q");
}

#[test]
fn embedded_json_quotes_do_not_break_columns() {
    // Doubled quotes inside a quoted field (common in JSON payload columns).
    let (doc, _d) = parse_bytes(b"id,payload\n1,\"{\"\"k\"\":1,\"\"v\"\":\"\"US\"\"}\"\n");
    assert_eq!(doc.column_count(), 2);
    assert_eq!(doc.record_count(), 1);
    assert_eq!(cell(&doc, 0, 1), "{\"k\":1,\"v\":\"US\"}");
}

#[test]
fn unterminated_quote_is_flagged() {
    let (doc, _d) = parse_bytes(b"a,b\n1,\"unterminated\n2,3\n");
    let flagged = doc
        .meta
        .warnings
        .iter()
        .any(|w| w.kind == anvil_core::WarningKind::UnterminatedQuote);
    assert!(flagged, "expected an unterminated-quote warning");
}

#[test]
fn unterminated_quote_index_points_at_the_partial_record() {
    // Regression: the warning used to subtract one from the record index,
    // so once earlier rows had completed it pointed at the previous row.
    let (doc, _d) = parse_bytes(b"a,b\n1,2\n3,\"unterminated\n4,5\n");
    let warning = doc
        .meta
        .warnings
        .iter()
        .find(|w| w.kind == anvil_core::WarningKind::UnterminatedQuote)
        .expect("unterminated-quote warning");
    assert_eq!(
        warning.record_index,
        Some(1),
        "the quoted partial record is record index 1"
    );
    assert_eq!(doc.record_count(), 2);
}

// ---- open_path dispatch -------------------------------------------------

#[test]
fn open_path_detects_csv_directly() {
    let (_d, path) = write_temp("plain.csv", b"a,b\n1,2\n");
    match open_path(&path).expect("open") {
        OpenResult::Direct(opened) => {
            assert_eq!(opened.len(), 8);
        }
        OpenResult::Archive(_) => panic!("plain CSV was misdetected as archive"),
    }
}

#[test]
fn open_path_rejects_empty_file() {
    let (_d, path) = write_temp("empty.csv", b"");
    assert!(open_path(&path).is_err());
}

// ---- ZIP handling -------------------------------------------------------

#[test]
fn zip_with_single_csv_is_detected() {
    let (_d, path) = make_zip("one.zip", &[("data.csv", b"a,b\n1,2\n")]);
    match open_path(&path).expect("open zip") {
        OpenResult::Archive(source) => {
            assert_eq!(source.total_candidates(), 1);
            let chosen = source
                .single_usable()
                .expect("one usable")
                .entry_name
                .clone();
            let opened = source.extract_into_temp(&chosen).expect("extract");
            assert_eq!(opened.bytes(), b"a,b\n1,2\n");
        }
        OpenResult::Direct(_) => panic!("zip was opened as a plain file"),
    }
}

#[test]
fn zip_with_multiple_csvs_has_multiple_candidates() {
    let (_d, path) = make_zip(
        "many.zip",
        &[
            ("a.csv", b"x\n1\n"),
            ("b.csv", b"y\n2\n"),
            ("c.tsv", b"z\n3\n"),
        ],
    );
    match open_path(&path).expect("open zip") {
        OpenResult::Archive(source) => {
            assert_eq!(source.total_candidates(), 3);
            assert!(source.single_usable().is_none());
        }
        OpenResult::Direct(_) => panic!("zip was opened as a plain file"),
    }
}

#[test]
fn zip_ignores_macos_noise_and_nested_dirs() {
    let (_d, path) = make_zip(
        "noisy.zip",
        &[
            ("exports/real.csv", b"a,b\n1,2\n"),
            ("__MACOSX/._real.csv", b"\x00\x05\x16\x07junk"),
            (".DS_Store", b"Bud1"),
            ("docs/readme.md", b"# hello"),
        ],
    );
    match open_path(&path).expect("open zip") {
        OpenResult::Archive(source) => {
            assert_eq!(
                source.total_candidates(),
                1,
                "only real.csv should be usable"
            );
            assert_eq!(
                source.single_usable().unwrap().entry_name,
                "exports/real.csv"
            );
            let rejected = source
                .candidates
                .iter()
                .filter(|c| !c.usable)
                .map(|c| c.entry_name.clone())
                .collect::<Vec<_>>();
            assert!(rejected.iter().any(|n| n.contains("__MACOSX")));
            assert!(rejected.iter().any(|n| n == ".DS_Store"));
        }
        OpenResult::Direct(_) => panic!("zip was opened as a plain file"),
    }
}

#[test]
fn zip_without_supported_files_has_no_usable_entries() {
    let (_d, path) = make_zip(
        "empty.zip",
        &[
            ("readme.md", b"# nothing to see"),
            ("image.png", b"\x89PNG"),
        ],
    );
    match open_path(&path).expect("open zip") {
        OpenResult::Archive(source) => {
            assert_eq!(source.total_candidates(), 0);
            assert!(source.single_usable().is_none());
        }
        OpenResult::Direct(_) => panic!("zip was opened as a plain file"),
    }
}

#[test]
fn zip_path_traversal_is_rejected() {
    let (_d, path) = make_zip("evil.zip", &[("../escape.csv", b"a\n1\n")]);
    let result = open_path(&path);
    assert!(
        result.is_err(),
        "path traversal entry should be rejected, got {result:?}"
    );
}

#[test]
fn zip_duplicate_entry_names_do_not_panic() {
    // The `zip` writer refuses to create duplicate names, so build the
    // bytes by hand. Anvil only needs to handle it gracefully: either the
    // reader lists both, or it reports an error. It must never panic.
    let bytes = raw_zip(&[("same.csv", b"a\n1\n"), ("same.csv", b"b\n2\n")]);
    let (_d, path) = write_temp("dup.zip", &bytes);
    match open_path(&path) {
        // The `zip` reader may collapse duplicates; we only require that
        // the archive is still usable and nothing panics.
        Ok(OpenResult::Archive(source)) => assert!(!source.candidates.is_empty()),
        Ok(OpenResult::Direct(_)) => panic!("duplicate-name zip opened as a plain file"),
        Err(_) => {}
    }
}

/// Minimal STORED-method ZIP writer, used only to produce archives the
/// `zip` crate's writer refuses to create (duplicate names, odd paths).
fn raw_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let owned: Vec<(&str, &[u8], u16, u32)> = entries
        .iter()
        .map(|(name, data)| (*name, *data, 0u16, data.len() as u32))
        .collect();
    raw_zip_impl(&owned)
}

/// Like `raw_zip`, but sets the same general-purpose flags (e.g. bit 0,
/// the encryption bit) on every entry.
fn raw_zip_with_flags(entries: &[(&str, &[u8])], flags: u16) -> Vec<u8> {
    let owned: Vec<(&str, &[u8], u16, u32)> = entries
        .iter()
        .map(|(name, data)| (*name, *data, flags, data.len() as u32))
        .collect();
    raw_zip_impl(&owned)
}

/// Like `raw_zip`, but declares a bogus uncompressed size in the headers
/// while storing only `data`. Used to exercise the compression-ratio guard
/// without building a genuine high-ratio payload.
fn raw_zip_with_declared_size(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let owned: Vec<(&str, &[u8], u16, u32)> = entries
        .iter()
        .map(|(name, data, declared)| (*name, *data, 0u16, *declared))
        .collect();
    raw_zip_impl(&owned)
}

fn raw_zip_impl(entries: &[(&str, &[u8], u16, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    for (name, data, flags, uncompressed) in entries {
        offsets.push(out.len() as u32);
        let crc = crc32fast::hash(data);
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // compressed
        out.extend_from_slice(&uncompressed.to_le_bytes()); // uncompressed
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
    }

    let central_start = out.len() as u32;
    let mut central = Vec::new();
    for (i, (name, data, flags, uncompressed)) in entries.iter().enumerate() {
        let crc = crc32fast::hash(data);
        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&flags.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // time
        central.extend_from_slice(&0u16.to_le_bytes()); // date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes()); // compressed
        central.extend_from_slice(&uncompressed.to_le_bytes()); // uncompressed
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offsets[i].to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);

    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // disk
    out.extend_from_slice(&0u16.to_le_bytes()); // central disk
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len
    out
}

#[test]
fn zip_encrypted_entry_is_skipped_and_reported() {
    // Bit 0 of the general-purpose flags marks an entry as encrypted. The
    // `zip` writer cannot produce this, so set the bit by hand.
    let bytes = raw_zip_with_flags(&[("secret.csv", b"a,b\n1,2\n")], 1);
    let (_d, path) = write_temp("locked.zip", &bytes);
    match open_path(&path).expect("open zip") {
        OpenResult::Archive(source) => {
            assert_eq!(source.total_candidates(), 0);
            let rejected = source
                .candidates
                .iter()
                .find(|c| c.entry_name == "secret.csv")
                .expect("candidate listed");
            assert!(!rejected.usable);
            assert_eq!(rejected.rejected_reason, anvil_core::REJECTED_ENCRYPTED);
            assert!(source.first_encrypted_entry().is_some());
        }
        OpenResult::Direct(_) => panic!("zip was opened as a plain file"),
    }

    // `load` turns "nothing usable, but it is encrypted" into the targeted
    // error so the UI can show a friendlier message than "no data files".
    let err = match anvil_core::load(&path) {
        Ok(_) => panic!("encrypted archive should not load"),
        Err(e) => e,
    };
    assert!(
        matches!(err, anvil_core::AnvilError::EncryptedEntry(ref name) if name == "secret.csv"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn zip_mixed_encrypted_and_plain_keeps_plain_usable() {
    let mixed = raw_zip_flagged_mixed(&[
        ("secret.csv", b"a,b\n1,2\n", 1u16),
        ("open.csv", b"x,y\n3,4\n", 0u16),
    ]);
    let (_d, path) = write_temp("mixed.zip", &mixed);
    match open_path(&path).expect("open zip") {
        OpenResult::Archive(source) => {
            assert_eq!(source.total_candidates(), 1);
            assert_eq!(source.single_usable().unwrap().entry_name, "open.csv");
            assert_eq!(
                source.first_encrypted_entry().unwrap().entry_name,
                "secret.csv"
            );
        }
        OpenResult::Direct(_) => panic!("zip was opened as a plain file"),
    }
}

/// Raw writer variant where each entry carries its own flags.
fn raw_zip_flagged_mixed(entries: &[(&str, &[u8], u16)]) -> Vec<u8> {
    let owned: Vec<(&str, &[u8], u16, u32)> = entries
        .iter()
        .map(|(name, data, flags)| (*name, *data, *flags, data.len() as u32))
        .collect();
    raw_zip_impl(&owned)
}

#[test]
fn zip_ratio_capped_entry_is_skipped() {
    // Store a tiny payload but declare a ~1 GiB uncompressed size. The
    // ratio guard should reject it before any extraction is attempted.
    let bytes = raw_zip_with_declared_size(&[("bomb.csv", b"a,b\n1,2\n", 1024 * 1024 * 1024)]);
    let (_d, path) = write_temp("bomb.zip", &bytes);
    match open_path(&path).expect("open zip") {
        OpenResult::Archive(source) => {
            assert_eq!(source.total_candidates(), 0);
            let rejected = source
                .candidates
                .iter()
                .find(|c| c.entry_name == "bomb.csv")
                .expect("candidate listed");
            assert!(!rejected.usable);
            assert!(
                rejected.rejected_reason.contains("compression ratio"),
                "unexpected reason: {}",
                rejected.rejected_reason
            );
        }
        OpenResult::Direct(_) => panic!("zip was opened as a plain file"),
    }
}

#[test]
fn corrupted_zip_reports_error_not_panic() {
    let (_d, path) = write_temp("broken.zip", b"PK\x03\x04this is not a real zip");
    // Should return Err (or an archive with no usable entries) -- never panic.
    let _ = open_path(&path);
}

#[test]
fn large_compressed_csv_round_trips() {
    let rows = 50_000;
    let mut csv = String::with_capacity(rows * 20);
    csv.push_str("id,payload\n");
    for i in 0..rows {
        // Highly compressible repetitive content.
        csv.push_str(&format!("{i},aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"));
    }
    let (_d, path) = make_zip("big.zip", &[("big.csv", csv.as_bytes())]);
    match open_path(&path).expect("open zip") {
        OpenResult::Archive(source) => {
            let entry = source.single_usable().unwrap().entry_name.clone();
            let opened = source.extract_into_temp(&entry).expect("extract");
            let result = parse_opened(&opened);
            let doc = ParsedDocument {
                opened,
                meta: result.meta,
                record_offsets: result.record_offsets,
            };
            assert_eq!(doc.record_count(), rows as u64);
            assert_eq!(cell(&doc, rows as u64 - 1, 0), (rows - 1).to_string());
        }
        OpenResult::Direct(_) => panic!("zip was opened as a plain file"),
    }
}

/// Measures indexing throughput against the generated fixture. Ignored by
/// default because it depends on `fixtures/large.csv` existing.
///
/// Run with: cargo test --release --test core -- --ignored measure_large
#[test]
#[ignore]
fn measure_large_fixture() {
    let path = std::path::Path::new("fixtures/large.csv");
    if !path.exists() {
        eprintln!("fixtures/large.csv not present; run gen_fixture first");
        return;
    }
    let opened = open_local(path).expect("open fixture");
    let size = opened.len();
    let started = std::time::Instant::now();
    let result = parse_opened(&opened);
    let elapsed = started.elapsed();
    let doc = ParsedDocument {
        opened,
        meta: result.meta,
        record_offsets: result.record_offsets,
    };
    let mb = size as f64 / (1024.0 * 1024.0);
    let mb_per_s = mb / elapsed.as_secs_f64();
    eprintln!(
        "indexed {:.1} MiB / {} rows in {:.0?} ({:.0} MiB/s), {} cols",
        mb,
        doc.record_count(),
        elapsed,
        mb_per_s,
        doc.column_count()
    );
    assert_eq!(doc.record_count(), 1_000_000);
    assert_eq!(doc.column_count(), 20);
}

/// Proxy for the grid's per-frame visible-cell text work: extract and
/// allocate the text for a viewport-sized window of cells (single
/// `Cells` pass per row, matching `ui::grid::render_row`), repeatedly.
/// This is not a GPU frame time because no window is created; it
/// isolates the allocation the renderer performs for every visible
/// cell.
///
/// It measures three forms over a one-row-per-frame scroll over the
/// 1M-row fixture so the Milestone 5 change can be judged directly:
///   1. `exact_string(bytes).to_string()` - the old render form
///      (intermediate `String` plus the `SharedString`/`Arc` copy).
///   2. `Arc::<str>::from(exact_string(bytes).as_ref())` - the new
///      production form, one copy.
///   3. a bounded `HashMap<(row, col), Arc<str>>` cache that persists
///      across frames (the render path keeps ~29 of 30 rows per scroll
///      step, so most lookups hit).
///
/// Run with:
///   cargo test --release --test core -- --ignored measure_viewport --nocapture
#[test]
#[ignore]
fn measure_viewport_cell_text() {
    use std::collections::HashMap;
    use std::sync::Arc;

    let path = std::path::Path::new("fixtures/large.csv");
    if !path.exists() {
        eprintln!("fixtures/large.csv not present; run gen_fixture first");
        return;
    }
    let opened = open_local(path).expect("open fixture");
    let result = parse_opened(&opened);
    let doc = ParsedDocument {
        opened,
        meta: result.meta,
        record_offsets: result.record_offsets,
    };
    const VISIBLE_ROWS: u64 = 40;
    const VISIBLE_COLS: usize = 20;
    const FRAMES: u64 = 1000;
    const CACHE_CAP: usize = 4096;
    let max_start = doc.record_count() - VISIBLE_ROWS;
    let cells_per_frame = VISIBLE_ROWS * VISIBLE_COLS as u64;

    // Form 1: old `to_string()` path.
    let mut written = 0usize;
    let started = std::time::Instant::now();
    for frame in 0..FRAMES {
        let first = frame % max_start;
        for row in first..first + VISIBLE_ROWS {
            let Some(record) = doc.record_bytes(row) else {
                continue;
            };
            let mut cells = Cells::new(record, doc.meta.delimiter, doc.meta.quote);
            for _ in 0..VISIBLE_COLS {
                let cell = cells.next();
                let bytes: &[u8] = match &cell {
                    Some(cell) => cell.as_ref(),
                    None => &[],
                };
                written += Arc::<str>::from(exact_string(bytes).to_string()).len();
            }
        }
    }
    let old = started.elapsed();

    // Form 2: new one-copy `Arc<str>` path (no intermediate String).
    let started = std::time::Instant::now();
    for frame in 0..FRAMES {
        let first = frame % max_start;
        for row in first..first + VISIBLE_ROWS {
            let Some(record) = doc.record_bytes(row) else {
                continue;
            };
            let mut cells = Cells::new(record, doc.meta.delimiter, doc.meta.quote);
            for _ in 0..VISIBLE_COLS {
                let cell = cells.next();
                let bytes: &[u8] = match &cell {
                    Some(cell) => cell.as_ref(),
                    None => &[],
                };
                written += Arc::<str>::from(exact_string(bytes).as_ref()).len();
            }
        }
    }
    let new = started.elapsed();

    // Form 3: bounded cache that persists across frames. The iterator is
    // always advanced so the cell lookup stays aligned on hits.
    let mut cache: HashMap<u64, Arc<str>> = HashMap::new();
    let started = std::time::Instant::now();
    for frame in 0..FRAMES {
        let first = frame % max_start;
        for row in first..first + VISIBLE_ROWS {
            let Some(record) = doc.record_bytes(row) else {
                continue;
            };
            let mut cells = Cells::new(record, doc.meta.delimiter, doc.meta.quote);
            for col in 0..VISIBLE_COLS {
                let cell = cells.next();
                let bytes: &[u8] = match &cell {
                    Some(cell) => cell.as_ref(),
                    None => &[],
                };
                let key = row * VISIBLE_COLS as u64 + col as u64;
                if let Some(cached) = cache.get(&key) {
                    written += cached.len();
                    continue;
                }
                let value: Arc<str> = Arc::from(exact_string(bytes).as_ref());
                written += value.len();
                if cache.len() >= CACHE_CAP {
                    cache.clear();
                }
                cache.insert(key, value);
            }
        }
    }
    let cached = started.elapsed();

    let report = |name: &str, elapsed: std::time::Duration| {
        eprintln!(
            "{name}: {} cells in {:.0?} ({:.1} ns/cell, {:.3} ms/frame)",
            FRAMES * cells_per_frame,
            elapsed,
            elapsed.as_nanos() as f64 / (FRAMES * cells_per_frame) as f64,
            elapsed.as_secs_f64() * 1000.0 / FRAMES as f64,
        );
    };
    report("to_string (old)", old);
    report("Arc<str> (new)", new);
    report("bounded cache ", cached);
    let _ = written;
}

/// Throughput of a text filter over the generated 1M-row fixture.
///
/// Run with: cargo test --release --test core -- --ignored measure_filter
#[test]
#[ignore]
fn measure_filter_large_fixture() {
    let path = std::path::Path::new("fixtures/large.csv");
    if !path.exists() {
        eprintln!("fixtures/large.csv not present; run gen_fixture first");
        return;
    }
    let opened = open_local(path).expect("open fixture");
    let result = parse_opened(&opened);
    let doc = ParsedDocument {
        opened,
        meta: result.meta,
        record_offsets: result.record_offsets,
    };
    // `country == US` is roughly 10% of the generated rows.
    let filter =
        parse_filter("country == US", &doc.meta.headers, doc.column_count()).expect("parse filter");
    let started = std::time::Instant::now();
    let matches = apply_filter(&doc, &[filter]);
    let elapsed = started.elapsed();
    eprintln!(
        "filtered {} rows -> {} matches in {:.0?}",
        doc.record_count(),
        matches.len(),
        elapsed
    );
    assert!(!matches.is_empty());
    assert!(elapsed.as_secs_f64() < 2.0, "filter took {elapsed:?}");
}

/// Throughput of a case-insensitive whole-record search over the
/// generated 1M-row fixture, which is Cmd/Ctrl+F's default path.
///
/// Run with: cargo test --release --test core -- --ignored measure_search
#[test]
#[ignore]
fn measure_search_large_fixture() {
    let path = std::path::Path::new("fixtures/large.csv");
    if !path.exists() {
        eprintln!("fixtures/large.csv not present; run gen_fixture first");
        return;
    }
    let opened = open_local(path).expect("open fixture");
    let result = parse_opened(&opened);
    let doc = ParsedDocument {
        opened,
        meta: result.meta,
        record_offsets: result.record_offsets,
    };
    // Default options are case-insensitive; "us" hits the uppercase
    // `US` country values, forcing the fold path.
    let opts = SearchOptions::default();
    let started = std::time::Instant::now();
    let matches = search_records(&doc, "us", &opts);
    let elapsed = started.elapsed();
    eprintln!(
        "searched {} rows (case-insensitive) -> {} matches in {:.0?}",
        doc.record_count(),
        matches.len(),
        elapsed
    );
    assert!(!matches.is_empty());
}

/// Throughput of the autocomplete value sampler over the fixture.
///
/// Run with: cargo test --release --test core -- --ignored measure_sample
#[test]
#[ignore]
fn measure_sample_distinct_large_fixture() {
    let path = std::path::Path::new("fixtures/large.csv");
    if !path.exists() {
        eprintln!("fixtures/large.csv not present; run gen_fixture first");
        return;
    }
    let opened = open_local(path).expect("open fixture");
    let result = parse_opened(&opened);
    let doc = ParsedDocument {
        opened,
        meta: result.meta,
        record_offsets: result.record_offsets,
    };
    let column = doc
        .meta
        .headers
        .iter()
        .position(|h| h == "country")
        .expect("country column");
    let started = std::time::Instant::now();
    let values = anvil_core::sample_distinct(&doc, column, 50_000, 1000);
    let elapsed = started.elapsed();
    eprintln!(
        "sampled {} distinct `country` values in {:.0?}",
        values.len(),
        elapsed
    );
    assert!(!values.is_empty());
}

/// Throughput + correctness of a numeric sort over the generated
/// 1M-row fixture, plus the inferred column labels.
///
/// Run with: cargo test --release --test core -- --ignored measure_sort
#[test]
#[ignore]
fn measure_sort_large_fixture() {
    let path = std::path::Path::new("fixtures/large.csv");
    if !path.exists() {
        eprintln!("fixtures/large.csv not present; run gen_fixture first");
        return;
    }
    let opened = open_local(path).expect("open fixture");
    let result = parse_opened(&opened);
    let doc = ParsedDocument {
        opened,
        meta: result.meta,
        record_offsets: result.record_offsets,
    };

    let profiles = infer_schema(&doc);
    eprintln!(
        "inferred: {}",
        profiles
            .iter()
            .map(|p| format!("{}={:?}", p.name, p.column_type))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let column = profiles
        .iter()
        .position(|p| p.name == "amount")
        .or_else(|| profiles.iter().position(|p| p.column_type.is_numeric()))
        .expect("a numeric column");
    let spec = SortSpec {
        column,
        order: SortOrder::Ascending,
        kind: SortKind::for_column_type(profiles[column].column_type),
    };
    let rows: Vec<u64> = (0..doc.record_count()).collect();

    let started = std::time::Instant::now();
    let sorted = sort_rows(&doc, &rows, &spec);
    let elapsed = started.elapsed();
    eprintln!(
        "sorted {} rows by `{}` in {:.0?}",
        doc.record_count(),
        profiles[column].name,
        elapsed
    );
    assert_eq!(sorted.len(), 1_000_000);
    assert!(elapsed.as_secs_f64() < 3.0, "sort took {elapsed:?}");
}

#[test]
fn archive_auto_opens_csv_over_readme_txt() {
    let (_d, path) = make_zip(
        "reads.zip",
        &[
            ("data.csv", b"a,b\n1,2\n"),
            ("README.txt", b"just some notes\n"),
        ],
    );
    match anvil_core::load(&path).expect("load") {
        LoadResult::Ready(loaded) => {
            assert_eq!(loaded.entry.as_deref(), Some("data.csv"));
            assert_eq!(loaded.document.record_count(), 1);
        }
        LoadResult::NeedArchiveChoice(_) => {
            panic!("a single CSV plus a README should auto-open the CSV")
        }
    }
}

#[test]
fn archive_with_two_csvs_requires_a_choice() {
    let (_d, path) = make_zip("two.zip", &[("a.csv", b"x\n1\n"), ("b.csv", b"y\n2\n")]);
    match anvil_core::load(&path).expect("load") {
        LoadResult::NeedArchiveChoice(source) => assert_eq!(source.total_candidates(), 2),
        LoadResult::Ready(_) => panic!("two CSVs should require explicit selection"),
    }
}

#[test]
fn archive_with_no_data_reports_error() {
    let (_d, path) = make_zip("none.zip", &[("notes.md", b"# hi")]);
    assert!(anvil_core::load(&path).is_err());
}

#[test]
fn archive_source_lists_archive_name() {
    let (_d, path) = make_zip("named.zip", &[("x.csv", b"a\n1\n")]);
    let source = ArchiveSource::from_path(&path).expect("archive source");
    assert_eq!(source.archive_name(), "named.zip");
}

// ---- selection model (Milestone 2) --------------------------------------
//
// Selection state is independent of GPUI; these tests exercise the
// public surface end-to-end so the data-side contract is stable. The
// deeper unit tests live in `src/selection.rs`.

#[test]
fn selection_starts_empty_and_round_trips_through_set() {
    let mut s = Selection::empty();
    assert!(s.is_empty());
    s.set(CellPos::new(4, 7));
    assert!(!s.is_empty());
    assert_eq!(s.cursor(), Some(CellPos::new(4, 7)));
    assert_eq!(s.cursor_column(), Some(ColumnIndex(7)));
}

#[test]
fn selection_extend_to_normalises_anchor_and_cursor() {
    let mut s = Selection::empty();
    s.set(CellPos::new(1, 1));
    s.extend_to(CellPos::new(5, 4));
    assert_eq!(s.anchor(), Some(CellPos::new(1, 1)));
    assert_eq!(s.cursor(), Some(CellPos::new(5, 4)));
    assert_eq!(
        s.normalize(),
        Some((CellPos::new(1, 1), CellPos::new(5, 4)))
    );
    // The anchor is preserved on subsequent `extend_to` calls: the
    // cursor moves while the anchor stays at (1,1), and the rectangle
    // is recomputed from the (anchor, cursor) pair.
    s.extend_to(CellPos::new(0, 0));
    assert_eq!(s.anchor(), Some(CellPos::new(1, 1)));
    assert_eq!(s.cursor(), Some(CellPos::new(0, 0)));
    assert_eq!(
        s.normalize(),
        Some((CellPos::new(0, 0), CellPos::new(1, 1)))
    );
}

#[test]
fn selection_move_cursor_clamps_inside_a_real_document() {
    let (doc, _d) = parse_bytes(b"a,b,c\n1,2,3\n4,5,6\n7,8,9\n");
    let mut s = Selection::empty();
    s.set(CellPos::new(0, 0));
    // Walk off the top-left edge; both deltas saturate at zero.
    s.move_cursor(-100, -100, doc.column_count(), doc.record_count(), false);
    assert_eq!(s.cursor(), Some(CellPos::new(0, 0)));
    // Walk past the bottom-right edge; clamps to last record / column.
    s.move_cursor(100, 100, doc.column_count(), doc.record_count(), false);
    assert_eq!(s.cursor(), Some(CellPos::new(2, 2)));
}

#[test]
fn selection_shift_arrow_keeps_anchor_intact() {
    let (doc, _d) = parse_bytes(b"a,b,c,d\n1,2,3,4\n5,6,7,8\n");
    let mut s = Selection::empty();
    s.set(CellPos::new(0, 0));
    s.move_cursor(0, 2, doc.column_count(), doc.record_count(), true);
    assert_eq!(s.anchor(), Some(CellPos::new(0, 0)));
    assert_eq!(s.cursor(), Some(CellPos::new(0, 2)));
    assert!(s.is_range());
    assert!(s.contains(0, 1));
    assert!(!s.contains(1, 0));
}

#[test]
fn selection_clear_removes_cursor_and_anchor() {
    let mut s = Selection::empty();
    s.set(CellPos::new(2, 3));
    s.extend_to(CellPos::new(4, 5));
    s.clear();
    assert!(s.is_empty());
    assert_eq!(s.cursor(), None);
    assert_eq!(s.anchor(), None);
    assert!(s.normalize().is_none());
}

#[test]
fn selection_contains_inside_a_multi_record_range() {
    let mut s = Selection::empty();
    s.set(CellPos::new(3, 3));
    s.extend_to(CellPos::new(1, 1));
    assert!(s.contains(1, 1));
    assert!(s.contains(2, 2));
    assert!(s.contains(3, 3));
    assert!(!s.contains(0, 0));
    assert!(!s.contains(4, 4));
    assert!(!s.contains(2, 0));
}

// ---- selection copy (Milestone 2, clipboard slice) ----------------------
//
// These tests exercise `Selection::copy_text_with` against real
// `ParsedDocument`s end-to-end. They are the data-side contract for
// "Cmd/Ctrl+C copies the selection as TSV". The actual clipboard write
// (`arboard`) is exercised manually from the GUI and is not unit-tested
// here because it requires a window server.

/// Cell lookup helper mirroring what the UI does at copy time.
fn copy_text_for(doc: &anvil_core::ParsedDocument, s: &Selection) -> String {
    s.copy_text_with(|r, c| doc.cell(r, ColumnIndex(c)).map(|c| c.into_string()))
}

#[test]
fn copy_text_empty_selection_returns_empty_string() {
    let (doc, _d) = parse_bytes(b"a,b\n1,2\n");
    let s = Selection::empty();
    assert_eq!(copy_text_for(&doc, &s), "");
}

#[test]
fn copy_text_single_cell_in_real_document() {
    let (doc, _d) = parse_bytes(b"a,b,c\n1,2,3\n4,5,6\n");
    let mut s = Selection::empty();
    s.set(CellPos::new(1, 2));
    let ending = if cfg!(windows) { "\r\n" } else { "\n" };
    assert_eq!(copy_text_for(&doc, &s), format!("6{ending}"));
}

#[test]
fn copy_text_rectangle_in_real_document() {
    let (doc, _d) = parse_bytes(b"a,b,c\n1,2,3\n4,5,6\n7,8,9\n");
    let mut s = Selection::empty();
    s.set(CellPos::new(0, 0));
    s.extend_to(CellPos::new(1, 1));
    let ending = if cfg!(windows) { "\r\n" } else { "\n" };
    assert_eq!(copy_text_for(&doc, &s), format!("1\t2{ending}4\t5{ending}"));
}

#[test]
fn copy_text_normalises_anchor_after_cursor() {
    // User clicks (1, 2), shift-clicks (0, 0): the rectangle is still
    // (0, 0)-(1, 2). Copy should reflect that rectangle, not the raw
    // anchor/cursor order.
    let (doc, _d) = parse_bytes(b"a,b,c\n1,2,3\n4,5,6\n7,8,9\n");
    let mut s = Selection::empty();
    s.set(CellPos::new(1, 2));
    s.extend_to(CellPos::new(0, 0));
    let ending = if cfg!(windows) { "\r\n" } else { "\n" };
    assert_eq!(
        copy_text_for(&doc, &s),
        format!("1\t2\t3{ending}4\t5\t6{ending}")
    );
}

#[test]
fn copy_text_preserves_embedded_newline_in_quoted_cell() {
    // The parser decodes quoted newlines; the formatter must quote
    // the cell back so paste-into-spreadsheet preserves the content.
    let (doc, _d) = parse_bytes(b"a,b\n\"line1\nline2\",z\n");
    let mut s = Selection::empty();
    s.set(CellPos::new(0, 0));
    let ending = if cfg!(windows) { "\r\n" } else { "\n" };
    assert_eq!(copy_text_for(&doc, &s), format!("\"line1\nline2\"{ending}"));
}

#[test]
fn copy_text_preserves_doubled_quotes_in_quoted_cell() {
    let (doc, _d) = parse_bytes(b"a,b\n\"say \"\"hi\"\"\",z\n");
    let mut s = Selection::empty();
    s.set(CellPos::new(0, 0));
    let ending = if cfg!(windows) { "\r\n" } else { "\n" };
    assert_eq!(
        copy_text_for(&doc, &s),
        format!("\"say \"\"hi\"\"\"{ending}")
    );
}

#[test]
fn copy_text_missing_cells_are_empty_strings() {
    // Row 1 has only two columns but the document declares three
    // (from headers). The missing third cell should be emitted as
    // empty, not panicking.
    let (doc, _d) = parse_bytes(b"a,b,c\n1,2,3\n4,5\n");
    let mut s = Selection::empty();
    s.set(CellPos::new(0, 0));
    s.extend_to(CellPos::new(1, 2));
    let ending = if cfg!(windows) { "\r\n" } else { "\n" };
    assert_eq!(
        copy_text_for(&doc, &s),
        format!("1\t2\t3{ending}4\t5\t{ending}")
    );
}

#[test]
fn copy_text_cell_count_matches_rectangle_size() {
    let (doc, _d) = parse_bytes(b"a,b,c,d\n1,2,3,4\n5,6,7,8\n9,10,11,12\n");
    let mut s = Selection::empty();
    s.set(CellPos::new(0, 1));
    s.extend_to(CellPos::new(2, 3));
    assert_eq!(s.cell_count(), 3 * 3);
    let ending = if cfg!(windows) { "\r\n" } else { "\n" };
    assert_eq!(
        copy_text_for(&doc, &s),
        format!("2\t3\t4{ending}6\t7\t8{ending}10\t11\t12{ending}")
    );
}

// ---- search ------------------------------------------------------------

#[test]
fn search_finds_matches_across_columns() {
    let (doc, _d) = parse_bytes(b"a,b,c\nfoo,bar,baz\nfoo,zip,foo\nzzz,foo,qqq\n");
    let opts = SearchOptions::default();
    let matches = search_records(&doc, "foo", &opts);
    assert_eq!(matches, vec![0, 1, 2]);
}

#[test]
fn search_returns_matches_in_record_order() {
    let (doc, _d) = parse_bytes(b"a,b\nfoo,1\nfoo,2\nfoo,3\n");
    let opts = SearchOptions::default();
    let matches = search_records(&doc, "foo", &opts);
    assert_eq!(matches, vec![0, 1, 2]);
}

#[test]
fn search_empty_needle_returns_no_matches() {
    let (doc, _d) = parse_bytes(b"a,b\n1,2\n3,4\n");
    let matches = search_records(&doc, "", &SearchOptions::default());
    assert!(matches.is_empty());
}

#[test]
fn search_case_sensitive_distinguishes_case() {
    let (doc, _d) = parse_bytes(b"a,b\nFoo,1\nfoo,2\nFOO,3\n");
    let cs = SearchOptions {
        case_sensitive: true,
        ..Default::default()
    };
    let ci = SearchOptions::default();
    // Case-sensitive "foo" only matches the middle row.
    assert_eq!(search_records(&doc, "foo", &cs), vec![1]);
    // Case-insensitive matches all three.
    assert_eq!(search_records(&doc, "foo", &ci), vec![0, 1, 2]);
}

#[test]
fn search_column_scope_limits_search() {
    let (doc, _d) = parse_bytes(b"name,city\nalice,paris\nbob,paris\n");
    let col0 = SearchOptions {
        column: Some(0),
        ..Default::default()
    };
    let col1 = SearchOptions {
        column: Some(1),
        ..Default::default()
    };
    assert_eq!(search_records(&doc, "alice", &col0), vec![0]);
    assert_eq!(search_records(&doc, "alice", &col1), Vec::<u64>::new());
    assert_eq!(search_records(&doc, "paris", &col1), vec![0, 1]);
}

#[test]
fn search_max_matches_caps_results() {
    let (doc, _d) = parse_bytes(b"a,b\nfoo,1\nfoo,2\nfoo,3\nfoo,4\nfoo,5\n");
    let opts = SearchOptions {
        max_matches: Some(2),
        ..Default::default()
    };
    assert_eq!(search_records(&doc, "foo", &opts), vec![0, 1]);
}

#[test]
fn search_handles_quoted_commas() {
    let (doc, _d) = parse_bytes(b"a,b\n\"foo, bar\",1\n");
    let matches = search_records(&doc, "foo, bar", &SearchOptions::default());
    assert_eq!(matches, vec![0]);
}

#[test]
fn search_finds_substring_match() {
    let (doc, _d) = parse_bytes(b"a,b\nfoobar,1\nfo,2\nbaz,3\n");
    let matches = search_records(&doc, "foo", &SearchOptions::default());
    assert_eq!(matches, vec![0]);
}

// ---- filter ------------------------------------------------------------

#[test]
fn filter_matches_by_column_name_and_operator() {
    let (doc, _d) = parse_bytes(b"country,amount\nUS,10\nUS,200\nUK,200\n");
    let country = parse_filter("country == US", &doc.meta.headers, doc.column_count())
        .expect("parse country filter");
    assert_eq!(apply_filter(&doc, &[country]), vec![0, 1]);

    let amount = parse_filter("amount > 100", &doc.meta.headers, doc.column_count())
        .expect("parse amount filter");
    assert_eq!(apply_filter(&doc, &[amount]), vec![1, 2]);
}

#[test]
fn filter_combines_predicates_with_and() {
    let (doc, _d) = parse_bytes(b"country,amount\nUS,10\nUS,200\nUK,200\n");
    let a = parse_filter("country == US", &doc.meta.headers, doc.column_count()).unwrap();
    let b = parse_filter("amount > 100", &doc.meta.headers, doc.column_count()).unwrap();
    assert_eq!(apply_filter(&doc, &[a, b]), vec![1]);
}

#[test]
fn filter_parse_error_is_reported() {
    let (doc, _d) = parse_bytes(b"a,b\n1,2\n");
    assert!(parse_filter("nope == 1", &doc.meta.headers, doc.column_count()).is_err());
    assert!(parse_filter("a bogus 1", &doc.meta.headers, doc.column_count()).is_err());
}

#[test]
fn filter_text_operators_are_case_insensitive() {
    let (doc, _d) = parse_bytes(b"name,id\nALICE,1\nbob,2\n");
    let f = parse_filter("name contains ali", &doc.meta.headers, doc.column_count()).unwrap();
    assert_eq!(apply_filter(&doc, &[f]), vec![0]);
}

// ---- load progress ------------------------------------------------------

/// Indexing reports monotonic, determinate progress and a final sample
/// that reaches the source length. The fixture is ~2.5 MiB so at least
/// one intermediate report (the step is 1 MiB) proves reporting happens
/// mid-scan rather than only at the ends.
#[test]
fn indexing_reports_monotonic_progress() {
    use std::sync::Mutex;

    let mut csv = String::from("a,b\n");
    for i in 0..300_000u32 {
        csv.push_str(&format!("{i},value-{i}\n"));
    }
    let (_dir, path) = write_temp("progress.csv", csv.as_bytes());
    let opened = open_local(&path).expect("open");

    let samples: Mutex<Vec<anvil_core::LoadProgress>> = Mutex::new(Vec::new());
    let sink = |p: anvil_core::LoadProgress| samples.lock().expect("lock").push(p);
    let doc = anvil_core::index_opened_with_progress(opened, Some(&sink));
    let samples = samples.into_inner().expect("lock");

    assert!(!samples.is_empty(), "indexing emitted no progress samples");
    assert!(samples
        .iter()
        .all(|s| s.phase == anvil_core::LoadPhase::Indexing));
    assert!(samples
        .iter()
        .all(|s| s.bytes_total == Some(doc.opened.len())));
    assert!(samples
        .windows(2)
        .all(|w| w[0].bytes_done <= w[1].bytes_done));
    let last = samples.last().expect("a sample");
    assert_eq!(last.bytes_done, doc.opened.len());
    assert!(
        samples.len() >= 2,
        "expected intermediate reports, got {}",
        samples.len()
    );
}

/// ZIP extraction reports monotonic determinate progress using the
/// central directory's uncompressed size, and the loader reports the
/// indexing phase afterwards. The entry is ~2.5 MiB so extraction emits
/// intermediate samples.
#[test]
fn zip_extraction_reports_progress() {
    use std::sync::Mutex;

    let mut csv = String::from("a,b\n");
    for i in 0..300_000u32 {
        csv.push_str(&format!("{i},value-{i}\n"));
    }
    let (_dir, path) = make_zip("progress.zip", &[("data.csv", csv.as_bytes())]);

    let samples: Mutex<Vec<anvil_core::LoadProgress>> = Mutex::new(Vec::new());
    let sink = |p: anvil_core::LoadProgress| samples.lock().expect("lock").push(p);
    let loaded = match anvil_core::load_with_progress(&path, Some(&sink)).expect("load") {
        LoadResult::Ready(loaded) => loaded,
        LoadResult::NeedArchiveChoice(_) => panic!("a single CSV should auto-open"),
    };
    let samples = samples.into_inner().expect("lock");

    let extracting: Vec<_> = samples
        .iter()
        .filter(|s| s.phase == anvil_core::LoadPhase::Extracting)
        .collect();
    assert!(
        extracting.len() >= 2,
        "expected intermediate extraction reports, got {}",
        extracting.len()
    );
    assert!(extracting
        .iter()
        .all(|s| s.bytes_total == Some(loaded.document.opened.len())));
    assert!(extracting
        .windows(2)
        .all(|w| w[0].bytes_done <= w[1].bytes_done));
    assert_eq!(
        extracting.last().expect("a sample").bytes_done,
        loaded.document.opened.len()
    );
    // The indexing phase is reported after extraction.
    assert!(samples
        .iter()
        .any(|s| s.phase == anvil_core::LoadPhase::Indexing));
}

/// No sink means no reporting cost and identical results.
#[test]
fn load_without_progress_still_succeeds() {
    let (_dir, path) = write_temp("noprogress.csv", b"a,b\n1,2\n");
    match anvil_core::load_with_progress(&path, None).expect("load") {
        LoadResult::Ready(loaded) => assert_eq!(loaded.document.record_count(), 1),
        LoadResult::NeedArchiveChoice(_) => panic!("direct file should be ready"),
    }
}

/// `exact_string_capped` keeps byte-faithful escaping for the prefix it
/// returns, backs off a UTF-8 cut to a character boundary, and reports
/// truncation.
#[test]
fn exact_string_capped_preserves_prefix_and_flags_truncation() {
    let (text, truncated) = anvil_core::exact_string_capped(b"hello", 10);
    assert_eq!(text.as_ref(), "hello");
    assert!(!truncated);

    // Exactly the cap is not truncated.
    let (text, truncated) = anvil_core::exact_string_capped(b"abc", 3);
    assert_eq!(text.as_ref(), "abc");
    assert!(!truncated);

    // A cut inside a multi-byte character backs off to its start.
    let emoji = "a\u{1F600}b";
    let (text, truncated) = anvil_core::exact_string_capped(emoji.as_bytes(), 2);
    assert!(truncated);
    assert_eq!(text.as_ref(), "a");

    // Invalid bytes keep the escaped form for the prefix.
    let bytes = [b'a', 0xff, b'b'];
    let (text, truncated) = anvil_core::exact_string_capped(&bytes, 2);
    assert!(truncated);
    assert_eq!(
        text.as_ref(),
        anvil_core::exact_string(&bytes[..2]).as_ref()
    );
    // The escaped prefix is valid UTF-8 text.
    assert!(std::str::from_utf8(text.as_ref().as_bytes()).is_ok());
}

/// `exact_string_window` pages by source offset, keeps a multibyte
/// character whole, and leaves an invalid slice unaligned.
#[test]
fn exact_string_window_pages_without_splitting_characters() {
    let full = anvil_core::exact_string_window(b"hello", 0, 10);
    assert_eq!(full.text.as_ref(), "hello");
    assert_eq!((full.start, full.end), (0, 5));
    assert!(!full.more_before && !full.more_after && !full.adjusted_boundary);

    let bytes = b"abcdefghij";
    let first = anvil_core::exact_string_window(bytes, 0, 4);
    assert_eq!(first.text.as_ref(), "abcd");
    assert!(first.more_after && !first.adjusted_boundary);
    let second = anvil_core::exact_string_window(bytes, first.end, 4);
    assert_eq!(second.text.as_ref(), "efgh");
    assert!(second.more_before);
    let third = anvil_core::exact_string_window(bytes, second.end, 4);
    assert_eq!(third.text.as_ref(), "ij");
    assert!(third.more_before && !third.more_after);

    // A cut inside U+1F600 backs the end off to the character before it.
    let emoji = "a\u{1F600}b";
    let cut = anvil_core::exact_string_window(emoji.as_bytes(), 0, 2);
    assert_eq!(cut.text.as_ref(), "a");
    assert_eq!((cut.start, cut.end), (0, 1));
    assert!(cut.adjusted_boundary && cut.more_after);
    let next = anvil_core::exact_string_window(emoji.as_bytes(), cut.end, 4);
    assert_eq!(next.text.as_ref(), "\u{1F600}");
    assert_eq!((next.start, next.end), (1, 5));

    // An offset on a continuation byte backs the start off, so the
    // character is shown whole and the previous page is not repeated
    // as a hole.
    let mid = anvil_core::exact_string_window(emoji.as_bytes(), 2, 8);
    assert_eq!(mid.start, 1);
    assert_eq!(mid.text.as_ref(), "\u{1F600}b");
    assert!(mid.adjusted_boundary && !mid.more_after);

    // A window that would otherwise be empty still includes the one
    // character the offset landed in.
    let tiny = anvil_core::exact_string_window(emoji.as_bytes(), 2, 1);
    assert_eq!(tiny.text.as_ref(), "\u{1F600}");
    assert!(tiny.adjusted_boundary);
    assert!(tiny.end > 2);

    // Invalid UTF-8 is escaped in place. The offsets stay where asked.
    let raw = [b'a', 0xff, b'b', b'c'];
    let invalid = anvil_core::exact_string_window(&raw, 1, 2);
    assert_eq!((invalid.start, invalid.end), (1, 3));
    assert!(!invalid.adjusted_boundary);
    assert!(invalid.more_before && invalid.more_after);
    assert_eq!(
        invalid.text.as_ref(),
        anvil_core::exact_string(&raw[1..3]).as_ref()
    );

    let past = anvil_core::exact_string_window(b"ab", 10, 4);
    assert_eq!((past.start, past.end), (2, 2));
    assert!(past.text.is_empty());
    assert!(past.more_before && !past.more_after);
}

// ---- UTF-8 BOM ----------------------------------------------------------

/// A UTF-8 BOM must never appear inside the first header's name, but it
/// must still be reported by the encoding diagnostics. No other cell
/// bytes change.
#[test]
fn utf8_bom_is_stripped_from_the_first_header_but_reported() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"name,age\nalice,30\nbob,40\n");
    let (doc, _dir) = parse_bytes(&bytes);

    assert_eq!(doc.meta.headers, vec!["name", "age"]);
    assert_eq!(cell(&doc, 0, 0), "alice");
    assert_eq!(cell(&doc, 1, 1), "40");
    // The BOM bytes stay in the source and diagnostics still flag them.
    assert!(anvil_core::scan_encoding(&doc).bom);
    // Header offset starts after the BOM.
    assert_eq!(doc.meta.header_range, Some((3, 11)));
}

/// A headerless file whose first byte is a BOM does not leak the BOM
/// into its first cell.
#[test]
fn utf8_bom_headerless_first_cell_is_clean() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"alice,30\ncarol,40\n");
    let (doc, _dir) = parse_bytes(&bytes);

    assert!(!doc.meta.has_headers);
    assert_eq!(cell(&doc, 0, 0), "alice");
    assert_eq!(cell(&doc, 0, 1), "30");
    assert!(anvil_core::scan_encoding(&doc).bom);
}

/// `bom_len` / `strip_utf8_bom` only react to a real BOM prefix.
#[test]
fn strip_utf8_bom_only_strips_the_bom() {
    assert_eq!(anvil_core::bom_len(b"abc"), 0);
    assert_eq!(anvil_core::strip_utf8_bom(b"abc"), b"abc");
    assert_eq!(anvil_core::bom_len(&[0xEF, 0xBB, 0xBF, b'a']), 3);
    assert_eq!(anvil_core::strip_utf8_bom(&[0xEF, 0xBB, 0xBF, b'a']), b"a");
    // A lone 0xEF is not a BOM.
    assert_eq!(anvil_core::strip_utf8_bom(&[0xEF, b'a']), &[0xEF, b'a']);
}

//! Integration tests for Anvil's data-side logic (no GPUI).
//!
//! These cover CSV/TSV parsing, quoting, indexing, diagnostics, and the
//! archive abstraction including ZIP safety checks.

use std::io::Write;
use std::path::PathBuf;

use anvil_core::source::local::open_local;
use anvil_core::{
    open_path, parse_opened, search_records, ArchiveSource, CellPos, ColumnIndex, LoadResult,
    OpenResult, OpenedSource, ParsedDocument, SearchOptions, Selection,
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
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    for (name, data) in entries {
        offsets.push(out.len() as u32);
        let crc = crc32fast::hash(data);
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
    }

    let central_start = out.len() as u32;
    let mut central = Vec::new();
    for (i, (name, data)) in entries.iter().enumerate() {
        let crc = crc32fast::hash(data);
        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // time
        central.extend_from_slice(&0u16.to_le_bytes()); // date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
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

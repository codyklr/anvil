//! Anvil binary entry point.
//!
//! Normal invocation launches the GPUI window. `anvil --probe <path>` runs
//! the same `anvil_core::load` path headlessly and prints a summary, which
//! is useful for CI and for verifying large fixtures without a GUI.

use anvil_core::{ColumnIndex, LoadResult, LoadedDocument};

mod ui;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--probe") {
        match args.get(pos + 1) {
            Some(path) => std::process::exit(probe(path)),
            None => {
                eprintln!("usage: anvil --probe <path>");
                std::process::exit(2);
            }
        }
    }
    ui::app::run();
}

/// Loads `path` exactly as the UI would and prints a summary. Returns the
/// process exit code.
fn probe(path: &str) -> i32 {
    let started = std::time::Instant::now();
    match anvil_core::load(std::path::Path::new(path)) {
        Ok(LoadResult::Ready(loaded)) => {
            print_loaded(&loaded, started.elapsed());
            0
        }
        Ok(LoadResult::NeedArchiveChoice(source)) => {
            println!("archive: {}", source.archive_name());
            println!("{} candidate files:", source.total_candidates());
            for candidate in source.candidates.iter().filter(|c| c.usable) {
                println!(
                    "  {}  ({} compressed / {} uncompressed)",
                    candidate.entry_name, candidate.compressed_size, candidate.uncompressed_size
                );
            }
            println!("(multiple files; the GUI opens every usable entry as a tab)");
            0
        }
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

fn print_loaded(loaded: &LoadedDocument, elapsed: std::time::Duration) {
    let doc = &loaded.document;
    let meta = &doc.meta;
    println!("file:       {}", doc.opened.display_label());
    if let Some(entry) = &loaded.entry {
        println!("entry:      {entry}");
    }
    println!(
        "delimiter:  {} (0x{:02x})",
        ui::describe_delimiter(meta.delimiter),
        meta.delimiter
    );
    println!("headers:    {}", meta.has_headers);
    println!("columns:    {}", doc.column_count());
    println!("rows:       {}", doc.record_count());
    println!("bytes:      {}", doc.opened.len());
    println!("warnings:   {}", meta.warnings.len());
    for warning in meta.warnings.iter().take(5) {
        println!("  - [{}] {}", warning.kind, warning.note);
    }

    let show = meta.headers.iter().take(6).collect::<Vec<_>>();
    println!("header row: {:?}", show);
    if doc.record_count() > 0 {
        let first: Vec<String> = (0..doc.column_count().min(6))
            .map(|col| {
                doc.cell(0, ColumnIndex(col))
                    .map(|c| c.into_string())
                    .unwrap_or_default()
            })
            .collect();
        println!("first row:  {:?}", first);
        let last = doc.record_count() - 1;
        let last_vals: Vec<String> = (0..doc.column_count().min(6))
            .map(|col| {
                doc.cell(last, ColumnIndex(col))
                    .map(|c| c.into_string())
                    .unwrap_or_default()
            })
            .collect();
        println!("last row:   {:?}", last_vals);
    }
    println!("probe time: {:.0?}", elapsed);
}

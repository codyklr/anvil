//! Anvil binary entry point.
//!
//! Normal invocation launches the GPUI window. `anvil --probe <path>` runs
//! the same `anvil_core::load` path headlessly and prints a summary, which
//! is useful for CI and for verifying large fixtures without a GUI.

use anvil_core::{ColumnIndex, LoadResult, LoadedDocument};

mod ui;

#[cfg(feature = "nl-model")]
mod nlmodel;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(not(feature = "nl-gliner"))]
    if args
        .iter()
        .any(|a| a == "--setup-ask-model" || a == "--ask-probe")
    {
        eprintln!("This command requires a build with --features nl-gliner.");
        std::process::exit(2);
    }
    #[cfg(feature = "nl-gliner")]
    if args.iter().any(|a| a == "--setup-ask-model") {
        match anvil_core::gliner::setup() {
            Ok(dir) => println!("Ask model installed at {}", dir.display()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    #[cfg(feature = "nl-gliner")]
    if let Some(pos) = args.iter().position(|a| a == "--ask-probe") {
        let outcome = match (args.get(pos + 1), args.get(pos + 2)) {
            (Some(path), Some(phrase)) => ask_probe(path, phrase),
            _ => Err("usage: anvil --ask-probe <path> <quoted question>".into()),
        };
        if let Err(error) = outcome {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
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

#[cfg(feature = "nl-gliner")]
fn ask_probe(path: &str, phrase: &str) -> Result<(), String> {
    let LoadResult::Ready(loaded) =
        anvil_core::load(std::path::Path::new(path)).map_err(|e| e.to_string())?
    else {
        return Err("Choose a single data file for the Ask probe.".into());
    };
    let columns = anvil_core::infer_schema(&loaded.document)
        .into_iter()
        .map(|p| anvil_core::NlColumn {
            name: p.name,
            column_type: p.column_type,
        })
        .collect();
    let schema = anvil_core::NlSchema {
        columns,
        aliases: Vec::new(),
    };
    let start = std::time::Instant::now();
    let values = |column: usize| -> Vec<String> {
        anvil_core::sample_distinct(&loaded.document, column, 50_000, 1000)
    };
    let expr = anvil_core::gliner::Interpreter::default().interpret(phrase, &schema, &values)?;
    println!(
        "{}",
        anvil_core::describe_nl(&expr, &loaded.document.meta.headers)
    );
    println!("inference including load: {:?}", start.elapsed());
    Ok(())
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

    let encoding = anvil_core::scan_encoding(doc);
    println!("bom:        {}", encoding.bom);
    println!("invalid utf-8: {} cell(s)", encoding.total_invalid_cells());
    for column in encoding.columns.iter().take(5) {
        println!(
            "  - [{}] {} invalid cell(s)",
            column.name, column.invalid_cells
        );
    }

    let show = meta.headers.iter().take(6).collect::<Vec<_>>();
    println!("header row: {:?}", show);
    // Sample rows are shown byte-faithfully, like the grid: a cell that
    // is not valid UTF-8 prints in its escaped form rather than as "".
    if doc.record_count() > 0 {
        let first: Vec<String> = (0..doc.column_count().min(6))
            .map(|col| {
                doc.cell(0, ColumnIndex(col))
                    .map(|c| c.to_exact_string().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        println!("first row:  {:?}", first);
        let last = doc.record_count() - 1;
        let last_vals: Vec<String> = (0..doc.column_count().min(6))
            .map(|col| {
                doc.cell(last, ColumnIndex(col))
                    .map(|c| c.to_exact_string().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        println!("last row:   {:?}", last_vals);
    }
    println!("probe time: {:.0?}", elapsed);
}

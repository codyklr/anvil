# Anvil

A native, keyboard-friendly CSV/data-file viewer for inspecting large
integration exports. Built in Rust with [GPUI](https://gpui.rs) (the UI
framework behind Zed). No webview, no Electron.

## Status: Milestone 1

Anvil currently opens a CSV/TSV/PSV/delimited text file or a ZIP archive,
indexes it incrementally on a background thread, and renders a virtualized
grid with sticky headers and horizontal/vertical scrolling.

## Running

```sh
cargo run --release --bin anvil          # launch the GUI
cargo run --release --bin gen_fixture fixtures 1000000   # make test data
./target/release/anvil --probe fixtures/large.csv        # headless check
```

`--probe <path>` runs the exact same open/index path the UI uses and prints
a summary. It is useful for CI and for measuring large fixtures without a GUI.

## Architecture

The crate is split so that data logic never depends on GPUI:

```
src/
  lib.rs            anvil_core: data-side public API (no GPUI)
  error.rs          typed errors
  loader.rs         open orchestration: path -> browsable document
  source/           logical data-source abstraction
    mod.rs            DataSource, OpenedSource, MmapSource, ArchiveCandidate
    local.rs          direct filesystem files
    archive.rs        ZIP discovery, safety checks, extraction to temp
  parser/
    mod.rs            Cells iterator, Cell, ParsedDocument
    delimit.rs        quote-aware delimiter + header detection
    indexing.rs       record byte-offset index, diagnostics
  selection.rs      selection model (GPUI-free)
  query.rs          search + match-range helpers (GPUI-free)
  ui/               GPUI presentation (binary only)
    app.rs            window lifecycle
    view.rs           toolbar + tabs + file panel + grid + status
    commands.rs       keyboard chord -> action binding table
    grid.rs           column layout + row rendering helpers
    archive_picker.rs archive presentation helpers
    theme.rs          palette + metrics
  main.rs           binary entry: GUI, or `--probe`
  bin/gen_fixture.rs synthetic CSV + ZIP generator
```

## Data representation

Anvil never materialises a `Vec<Vec<String>>` of the file.

- The source file is memory-mapped (`memmap2`).
- Indexing walks the bytes once and records the **start byte offset of
  every record** in a `Vec<u64>` (the last entry is the file length, so
  every record has an exclusive end). Quotes are tracked with an explicit
  `at_field_start` flag so delimiters and newlines inside quoted fields are
  not misread.
- The grid asks for a specific `(row, column)`. The document slices the
  mmap for that record's byte range and runs a small state machine over it,
  returning a borrowed `Cow` for the cell. Nothing is retained per row.

For a 165 MiB file this index costs about **8 MB** of `u64`s and roughly
**250–340 ms** (≈470 MiB/s), on top of a page-cache-warm mmap.

## ZIP handling

`open_path` sniffs the first four bytes (without reading the whole file).
A ZIP becomes an `ArchiveSource` which:

1. lists entries and classifies each one,
2. rejects directories, `__MACOSX`, `.DS_Store`, hidden files, `..` paths,
   encrypted entries, unsupported compression methods, and oversized or
   zip-bomb-ratio entries,
3. ranks usable entries by extension (`csv`/`tsv` > `dat` > `txt`),
4. auto-opens the single best-ranked entry when there is a clear
   favourite, otherwise reports that the choice is ambiguous.

In the GUI, every usable entry becomes its own tab immediately (the
central directory already lists names and sizes). The entry the loader
pre-opened is ready; the others are extracted and indexed lazily when
their tab is first activated. Each entry is decompressed to a
`tempfile`, then memory-mapped and indexed through the *same* path as a
normal file. The temp directory is kept alive by an `Arc<TempDir>`
inside `OpenedSource` and removed on drop.

## Threading

Opening, extraction, and indexing run on GPUI's background executor. Each
open bumps a generation counter; a result whose generation is stale is
dropped, so dropping a new file while a large archive is still being indexed
is safe. The UI thread only ever renders the already-built index.

## Tests

```sh
cargo test        # 64 data-side + 56 binary (UI / command / geometry) tests
cargo test --release --test core -- --ignored measure_large   # throughput
```

Coverage includes normal/quoted/TSV/semicolon/pipe input, embedded newlines,
escaped quotes, missing and empty values, duplicate/empty headers,
inconsistent rows, extremely wide files, extremely long cells, headerless and
newline-less files, a 100k-row synthetic dataset, single/multi-file ZIPs,
nested paths, `__MACOSX`, corrupted ZIPs, path traversal, duplicate entry
names, and a large compressed CSV.

## Not yet done (Milestones 2–5)

Selection & keyboard navigation, clipboard, search, filtering, sorting, type
inference, column statistics, richer diagnostics, performance and UX
hardening. See `ROADMAP.md` for the planned work and acceptance criteria.

## Documentation

- `VISION.md` — durable product intent.
- `ROADMAP.md` — milestone-by-milestone plan; checkboxes reflect the
  actual repository state.
- `ARCHITECTURE.md` — current technical architecture.
- `AGENTS.md` — operating manual for coding agents working in this
  repository.
- `docs/agent-log.md` — chronological handoff log.
- `docs/prompts/initial-anvil-prompt.md` — historical product brief.

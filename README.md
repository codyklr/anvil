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
  schema.rs         heuristic per-column type inference
  schema/stats.rs   column statistics for the inspector
  query.rs          search + match-range helpers (GPUI-free)
  query/filter.rs   predicate filter
  query/sort.rs     column sort (keyed permutation)
  ui/               GPUI presentation (binary only)
    app.rs            window lifecycle
    view.rs           toolbar + tabs + file panel + grid + status
    view/search.rs     search bar (state, render, editing)
    view/filter.rs     filter bar (apply, render, editing)
    view/tests.rs      headless UI + unit test suite
    input.rs          shared editable-text rendering / hit-testing
    text_edit.rs      GPUI-free single-line edit buffer
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
cargo test        # 76 data-side + 136 binary (UI / command / geometry) + 1 style guard

# Throughput benchmarks (need fixtures/large.csv):
cargo test --release --test core -- --ignored measure_large   # indexing
cargo test --release --test core -- --ignored measure_search  # case-insensitive search
cargo test --release --test core -- --ignored measure_filter  # predicate filter
cargo test --release --test core -- --ignored measure_sort    # numeric sort
cargo test --release --bin anvil measure_wide_column_layout -- --ignored --nocapture
```

Coverage includes normal/quoted/TSV/semicolon/pipe input, embedded newlines,
escaped quotes, missing and empty values, duplicate/empty headers,
inconsistent rows, extremely wide files, extremely long cells, headerless and
newline-less files, a 100k-row synthetic dataset, single/multi-file ZIPs,
nested paths, `__MACOSX`, corrupted ZIPs, path traversal, duplicate entry
names, a large compressed CSV, compound filter expressions
(`AND` / `OR` / parentheses) with context-aware autocomplete, and
row-scope `any` / `all` predicates such as `any empty` (a row with a
blank cell) and `all notempty`.

## Natural-language Ask

`Cmd/Ctrl+Shift+L` opens an **Ask** bar. Type a phrase such as
`country is US and amount over 100`; a deterministic parser previews the
equivalent filter, and Enter hands it to the normal filter bar (where it
can still be edited). This works with no extra install, download, or
dependency. Row-scope questions work too: `at least one column is blank`
previews as `any empty`, `every column is filled` as `all notempty`, and
`no column is empty` as `all notempty`. Both word orders work, so
`any row that has an empty column` also previews as `any empty`.

### Native GLiNER extraction (optional)

GLiNER2.5 Small runs locally inside Anvil through Candle. It requires no
Python, PyTorch installation, server, or ONNX runtime. Install its roughly
296 MB weights plus tokenizer once, then launch the enabled build:

```sh
cargo run --release --features nl-gliner --bin anvil -- --setup-ask-model
cargo run --release --features nl-gliner --bin anvil
```

Open Ask with `Cmd/Ctrl+Shift+L`. When the rules cannot resolve a phrase,
press `Ctrl/Cmd+Enter` to interpret it, review the preview, then press
Enter to apply. Partial interpretations cannot be accepted. This first
backend supports **one condition**, such as `Last names that start with C`.
It also resolves indirect columns by sampled value: a question that never
names the column, such as `Which users are from Canada?`, binds `Canada`
to the column whose sampled values uniquely contain it (`country == Canada`),
and asks rather than guesses when two columns contain the value.
Use the filter bar for compound queries, aggregation, sorting, and dates.
Model predictions can be wrong; always review the preview.

The model stays loaded after first use. Inference runs on a background
executor and does not send questions or dataset contents over the network.
Only `--setup-ask-model` downloads files, from a pinned Hugging Face
revision. It reports file progress in the terminal; in-app download progress
and cancellation remain future work. Files use the platform cache directory
listed below, under a revision-specific GLiNER folder. `ANVIL_NL_CACHE`
overrides the cache root; `ANVIL_GLINER_MODEL_DIR` selects a checkpoint
directory containing `config.json`, `encoder_config/config.json`,
`tokenizer.json`, and `model.safetensors`.

Limits: 512 bytes per question, 32 columns, 1024 bytes of column names,
and 512 encoded tokens including the extraction schema. Oversized requests
are rejected without truncation. Low-confidence or invalid outputs report
an error. The single-condition checks are conservative, not a complete
English grammar; some valid requests need rephrasing.

For a repeatable real-model check (after setup):

```sh
cargo test --release --features nl-gliner real_model_filter_smoke -- --ignored --nocapture
cargo run --release --features nl-gliner --bin anvil -- --ask-probe fixtures/large.csv "amount exceeds 100"
```

`nl-gliner` takes precedence when both model features are enabled.
The community `gliner-rs` dependency is pinned to 0.2.3. It shares Candle
0.11 with the existing backend, but currently also compiles upstream CLI
dependencies; it is optional rather than dependency-free.

### Generative model (existing alternative)

The existing optional embedded generative model remains available:

```sh
cargo run --release --features nl-model --bin anvil
```

With that feature, `Ctrl/Cmd+Enter` in the Ask bar escalates an
unresolved phrase to a small local instruct model (default
Qwen2.5-Coder-1.5B-Instruct Q4_K_M, about 1.1 GB). The model is
downloaded once from the Hugging Face Hub on first use into the platform
cache directory (`~/Library/Caches/anvil/models` on macOS,
`%LOCALAPPDATA%\anvil\models` on Windows, `$XDG_CACHE_HOME/anvil/models`
or `~/.cache/anvil/models` on Linux), then runs entirely on CPU. Every
model result is re-validated by the filter parser before it can become a
query.

Both Llama-architecture (SmolLM2) and Qwen2-architecture (Qwen2.5) GGUFs
load: the family is read from the GGUF metadata. Prefer code-tuned or
strongly instruction-tuned instruct models in the 0.5B-1.5B range; keep
the standard ChatML prompt.

Overrides: `ANVIL_NL_MODEL_REPO` / `ANVIL_NL_MODEL_FILE`,
`ANVIL_NL_TOKENIZER_REPO` / `ANVIL_NL_TOKENIZER_FILE`, `ANVIL_NL_ENDPOINT`
(a mirror), `ANVIL_NL_CACHE` (cache location), and `HF_TOKEN` (for gated
models). For a lighter download, use Qwen2.5-Coder-0.5B-Instruct
(about 470 MB):

```sh
ANVIL_NL_MODEL_REPO=Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF \
ANVIL_NL_MODEL_FILE=qwen2.5-coder-0.5b-instruct-q4_k_m.gguf \
ANVIL_NL_TOKENIZER_REPO=Qwen/Qwen2.5-Coder-0.5B-Instruct \
  cargo run --release --features nl-model --bin anvil
```

The default build has none of these crates.

## Not yet done (Milestones 4–5)

Richer diagnostics (the panel lists parse warnings and encoding
findings, and the archive picker now lists skipped entries with their
reasons, including encrypted, unsupported-compression, and
high-compression-ratio entries; empty/duplicate-header navigation and
saved filters remain, and the filter language has no general `NOT` yet,
though row-scope `any` / `all` predicates (for example `any empty`) have
landed),
performance profiling, cross-platform hardening, and UX polish. See
`ROADMAP.md` for the planned work and acceptance criteria.

## Documentation

- `VISION.md` — durable product intent.
- `ROADMAP.md` — milestone-by-milestone plan; checkboxes reflect the
  actual repository state.
- `ARCHITECTURE.md` — current technical architecture.
- `AGENTS.md` — operating manual for coding agents working in this
  repository.
- `docs/agent-log.md` — chronological handoff log.
- `docs/prompts/initial-anvil-prompt.md` — historical product brief.

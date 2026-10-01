# Anvil Benchmarks

Performance numbers recorded on the reference machine. These are the
**"before"** figures for Milestone 5 work (per-cell allocation and
scroll profile); they are not aspirational targets.

## Reference machine

| | |
|---|---|
| CPU | Apple M4 (10 cores: 4 performance + 6 efficiency) |
| Memory | 24 GiB |
| OS | macOS 26 (Darwin 25.6.0), arm64 |
| Toolchain | rustc 1.98.1, `--release` (`opt-level = 3`, thin LTO, 1 codegen unit) |
| Build | `cargo build --release` from `2158e0e` + the CI and keyboard iterations |

All measurements are wall-clock on an otherwise idle machine. Fixtures
are generated with `gen_fixture`:

```sh
cargo run --release --bin gen_fixture -- fixtures 1000000
cargo run --release --bin gen_fixture -- fixtures/big 3000000
```

## Indexing and peak memory (`anvil --probe`)

Each row is the median of three runs of
`/usr/bin/time -l target/release/anvil --probe <fixture>`. `index time`
is the tool's own `probe time` (mmap + index + a first/last row sample),
not process startup. `peak RSS` is the kernel's `maximum resident set
size`.

| Fixture | Size | Rows | Cols | Index time | Peak RSS |
|---|---|---|---|---|---|
| `fixtures/ask.csv` | 4 KiB | 4 | 4 | 77 µs | 9 MiB |
| `fixtures/malformed.csv` | 4 KiB | 2 | 3 | 78 µs | 9 MiB |
| `fixtures/large.csv` | 165.4 MiB | 1,000,000 | 20 | 249 ms | 182 MiB |
| `fixtures/big/large.csv` | 498.3 MiB | 3,000,000 | 20 | 747 ms | 530 MiB |
| `fixtures/archive.zip` | 49.1 MiB (165.4 MiB entry) | 1,000,000 | 20 | ~1.0 s | 182 MiB |
| `fixtures/multi.zip` | few KiB | 2 entries | | < 1 ms (listing) | 9 MiB |

Index-only throughput on the 1M fixture, measured with
`cargo test --release --test core -- --ignored measure_large --nocapture`
(no probe sampling):

```
indexed 165.4 MiB / 1000000 rows in 254ms (651 MiB/s), 20 cols
```

The 3M fixture indexes at the same ~667 MiB/s (498.3 MiB / 747 ms).

### Reading the peak-RSS column

The indexer scans the whole memory-mapped file, so every page it touches
becomes resident and is counted in peak RSS. These pages are
**file-backed and reclaimable**, not anonymous allocations: the durable
per-document cost is the record-offset index (`record_count × 8` bytes:
8 MiB for 1M rows, 24 MiB for 3M), plus the mmap itself. That is why
peak RSS tracks the file size (165 MiB file → 182 MiB RSS; 498 MiB file
→ 530 MiB RSS) rather than the index size.

The ZIP row shows the same 182 MiB peak: the entry is decompressed to a
temp file and then memory-mapped and indexed through the identical path.

## Grid render proxy (per-cell text)

The grid cannot be driven interactively from a headless environment, so
no GPU frame time is recorded here. Instead,
`measure_viewport_cell_text` isolates the CPU work `ui::grid::render_row`
does for every visible cell: a single `Cells` pass per row, then the
text allocation. It measures 1,000 frames of a 40-row x 20-column
viewport scrolling one row per frame on the 1M fixture, and reports
three forms.

| Form | ns/cell | ms/frame | Notes |
|---|---|---|---|
| `exact_string(..).to_string()` + `SharedString` (old) | 81.9 | 0.066 | two copies: intermediate `String`, then `Arc<str>` |
| `Arc::<str>::from(exact_string(..).as_ref())` (new) | 53.4 | 0.043 | one copy |
| bounded visible-range cache (shipped) | 22.6 | 0.018 | allocation only for cells that just entered the view |

Iteration 4 reduced `ui/grid.rs::render_row` from the two-copy form to
the cached form: `GridModel` now holds a bounded
(`CELL_TEXT_CACHE_CAP = 4096`) `(record, column) -> SharedString` cache.
A scroll step keeps ~39 of 40 visible rows, so the cache removes
~95% of the per-cell `Arc<str>` allocations during scrolling and all of
them on a pure re-render (selection change, search highlight, hover).
The proxy shows **0.066 ms/frame -> 0.018 ms/frame (3.6x)** for the
allocation work alone. This is deliberately a *proxy*: it excludes GPUI
element construction, shaping, and GPU submission, which the change
does not affect.

Memory is bounded: 4096 entries of a typical short cell is a few
hundred KiB. A file wide enough to fill the cap clears it rather than
growing; for the common case (fewer than ~100 visible columns) it never
fills. The text depends only on the document, so a new `GridModel`
(open / reindex) is a complete invalidation - no staleness window.

## What is not measured

- **GPU scroll frame time.** The GUI was not launched in this
  environment; there is no interactive scroll profile yet. A frame timer
  or an instrumented build on a desktop is the next step for that item.
- **Filter / search / sort throughput.** The existing ignored tests
  (`measure_filter`, `measure_search`, `measure_sort`,
  `measure_sample`) already cover these; run them with
  `cargo test --release --test core -- --ignored measure_ --nocapture`.

# Anvil Vision

## What Anvil Is

Anvil is a native, high-performance data inspection tool written in Rust and GPUI.

It is designed for quickly opening, exploring, troubleshooting, and understanding large CSV, TSV, and other delimited datasets used in data integration work.

Anvil is not intended to be a spreadsheet replacement.

## Core Principles

- Extremely fast time-to-first-data
- Remain responsive with very large files
- Native desktop experience on Windows, macOS, and Linux
- Cross-platform behavior: core workflows and keyboard/clipboard
  interactions should work consistently on all three target OSes
- Keyboard-friendly workflows
- Treat archives such as ZIP files as first-class data sources
- Avoid unnecessary copies and memory usage
- Surface malformed or suspicious data instead of hiding it
- Optimize for investigation and troubleshooting

## Core Workflow

A user should be able to:

1. Launch Anvil.
2. Drag in a CSV, TSV, or ZIP export.
3. Immediately begin inspecting the data.
4. Search, filter, sort, and inspect columns.
5. Identify malformed records, unusual values, duplicates, and structural problems.
6. Understand the dataset without needing another tool.

## Archive Philosophy

ZIP files should behave like normal inputs.

Users should never need to manually extract an archive before opening it in Anvil.

If an archive contains one usable data file, Anvil should open it automatically.

If it contains multiple usable files, Anvil should provide an internal way to browse and switch between them.

## Performance Philosophy

Anvil should be designed around large datasets from the beginning.

Prefer:

- lazy work
- streaming where appropriate
- indexed row access
- bounded memory
- virtualized rendering
- asynchronous processing
- minimal allocation
- cancellation of obsolete work

Performance optimizations should be guided by measurement.

## Intended Features

### Data Sources
- CSV
- TSV
- configurable delimiters
- ZIP archives
- future support for additional compressed formats

### Data Inspection
- virtualized grid
- row and cell selection
- search
- filtering
- sorting
- column statistics
- type inference
- diagnostics

### Troubleshooting
- inconsistent row widths
- malformed quoting
- encoding problems
- duplicate headers
- empty headers
- unusual values
- duplicate identifiers
- archive problems

## Non-Goals

Anvil is not intended to become:

- Excel
- a full spreadsheet editor
- a database IDE
- a general ETL platform
- a web application

Editing may exist where useful, but inspection and analysis come first.

## Guiding Question

When evaluating a feature, ask:

> Does this make it faster or easier to understand what is actually inside this dataset?

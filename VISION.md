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
- Show data exactly as it exists: never transform, trim, normalise or
  truncate a value
- Be self-contained: everything Anvil downloads or persists (model
  files, caches, future configuration) lives in a folder beside the
  executable, so deleting the program's folder removes the program and
  everything it created. The only exception is transient ZIP
  extraction, which uses the OS temp directory and is cleaned up
- Optimize for investigation and troubleshooting

## Data Fidelity

Anvil shows data exactly as it exists in the file. A value is never
normalised, reformatted, rounded, re-encoded or truncated on its way to
the screen.

- Leading and trailing whitespace is preserved: `" 42 "` is displayed
  as `" 42 "`, not as `42`.
- Leading zeroes and trailing digits are preserved: `007` stays `007`
  and `1.10` stays `1.10`. Numeric text is never reformatted.
- Nothing is silently replaced. Bytes that are not valid UTF-8 are shown
  in an escaped, reversible form (`\xff`) and reported by the diagnostics
  panel, rather than being dropped or turned into a replacement
  character.
- Values are never truncated. Long values are elided in the grid only to
  fit the column, and the full value is always available in the column
  inspector and on copy.
- Values that differ only in padding or case are treated as the distinct
  values they are when values are counted or listed.

Where Anvil must interpret structure - the delimiter, CSV quoting, line
endings, the character encoding - it does so in order to reveal the field
value, and it reports what it found instead of rewriting it. That is why
the parser strips the record's line terminator and unescapes doubled
quotes, but never touches the bytes inside a field.

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

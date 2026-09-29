You are helping me design and build **Anvil**, a custom high-performance CSV/data-file viewer for my work.

Anvil should be a **native desktop application written in Rust using GPUI**. The goal is not to build a generic spreadsheet application. It should be an extremely fast tool for opening, inspecting, navigating, filtering, and understanding large CSV-style datasets during data integration and troubleshooting work.

I am running this through my **pi-openrouter setup using JEV for routing**, so treat this as a real engineering task rather than a one-shot code-generation request. Inspect the existing repository and environment first, make concrete implementation decisions, implement incrementally, and verify what you build.

## Product goal

Anvil should feel like a purpose-built native developer/data-inspection tool.

The core experience should be:

1. Launch Anvil.
2. Open or drag in a CSV, TSV, delimited text file, or ZIP archive containing data files.
3. See useful data almost immediately, even for very large files.
4. Scroll through rows and columns smoothly.
5. Search/filter/sort data without the UI becoming sluggish.
6. Inspect individual cells, columns, inferred types, malformed data, and unusual values.
7. Quickly answer questions like:
   - What does this file contain?
   - Which column corresponds to a particular field?
   - Are IDs duplicated?
   - Are there blank or malformed values?
   - What values occur in this column?
   - Which rows match a condition?
   - Does this file look structurally wrong?
   - How many records am I actually dealing with?
   - What files are contained in this uploaded archive?

The application should prioritize **speed, responsiveness, observability, and practical data investigation** over spreadsheet-style editing.

## Technology constraints

Use:

- **Rust**
- **GPUI** for the desktop UI
- Native Rust libraries wherever practical
- A modular architecture that keeps parsing/query logic separate from GPUI presentation logic

Avoid introducing a webview, Electron, Tauri, React, or an embedded browser stack.

Do not blindly load an entire huge CSV into an enormous matrix of Rust `String`s.

Design around large files from the beginning.

## Performance philosophy

Anvil should be capable of handling files that are large enough to make ordinary CSV viewers unpleasant.

Design toward:

- fast startup
- very fast time-to-first-rows
- streaming or incremental parsing where appropriate
- bounded memory usage
- virtualization of rows and columns
- asynchronous/background file processing
- cancellation of obsolete queries/tasks
- avoiding unnecessary allocations and copies
- zero-copy or borrowed parsing where it makes sense
- memory mapping where it provides a measurable benefit
- keeping expensive work off the UI thread

Do not prematurely write unsafe code.

Use profiling and measurement before introducing complex low-level optimizations.

Prefer an architecture where the UI can display an initial portion of the file while indexing/statistics continue in the background.

## File and archive handling

Anvil should make opening real-world data exports as frictionless as possible.

Support direct opening and drag/drop of:

- CSV
- TSV
- other delimited text files
- ZIP archives containing one or more supported data files

A user should **never need to manually extract a ZIP archive before opening it in Anvil**.

If a ZIP file is dragged into the application, Anvil should inspect and handle the archive itself.

### ZIP behavior

When opening a ZIP archive:

1. Read the archive directory.
2. Identify supported candidate data files.
3. Ignore obvious metadata and irrelevant system files such as:
   - `__MACOSX`
   - `.DS_Store`
   - directories
   - other obvious non-data files
4. If exactly one usable data file is present, open it automatically.
5. If multiple usable data files are present, show an internal file-selection interface inside Anvil.
6. Allow switching between files in the archive without requiring the archive to be reopened.
7. Show archive context somewhere in the UI, including:
   - archive filename
   - selected inner filename
   - compressed size where useful
   - uncompressed size where useful
   - number of candidate data files

The workflow should feel as though ZIP is simply another supported input format.

Do not tell the user to extract the archive manually.

### Archive extraction strategy

Avoid unnecessarily extracting an entire archive into a permanent folder.

Design an internal archive abstraction that can:

- inspect ZIP contents
- expose individual files to the parser
- extract only the selected entry when appropriate
- stream/decompress where practical
- use a temporary file where random access or memory mapping requires it
- clean up temporary files automatically

For large compressed CSVs, carefully evaluate the tradeoff between:

- streaming directly from the ZIP entry
- decompressing to a temporary file
- building an indexed temporary representation

Since CSV browsing requires random row access after initial parsing/indexing, it may be appropriate to transparently decompress a selected archive entry into a temporary file and then use the same indexing/data-access path as a normal file.

This should happen automatically and invisibly to the user.

Do not duplicate archive-handling logic throughout the application. Create a clean abstraction where the rest of Anvil largely works with a logical data source regardless of whether it originated as:

- a normal filesystem file
- a file inside a ZIP archive
- a temporary decompressed file

For example, consider abstractions conceptually similar to:

- `DataSource`
- `LocalFileSource`
- `ArchiveSource`
- `ArchiveEntry`
- `ResolvedDataSource`

Choose names and architecture appropriate to the codebase.

### ZIP safety

Treat archives as untrusted input.

Protect against:

- path traversal / zip-slip paths
- unexpectedly huge decompressed files
- malicious compression ratios / zip bombs
- corrupted archives
- encrypted ZIP entries
- unsupported compression methods
- duplicate archive entry names
- deeply nested paths where relevant

Never extract archive entries using their raw paths without validation.

Show useful errors inside Anvil rather than crashing.

Where sensible, expose archive warnings in the diagnostics UI.

### Future archive extensibility

Design the archive abstraction so additional formats could be supported later, such as:

- `.gz`
- `.tar.gz`
- `.7z`

Do not implement all of these during the initial milestone unless doing so is trivial.

ZIP is the required archive format for the initial implementation.

## File handling

Support:

- CSV
- TSV
- configurable delimiters
- automatic delimiter detection
- quoted fields
- escaped quotes
- embedded delimiters inside quoted fields
- embedded newlines where valid
- files with or without headers
- common text encodings where reasonably practical

The open-file experience should support:

- file picker
- drag and drop
- direct ZIP archive opening
- recent files eventually
- reopening the current file
- visible file metadata

Drag/drop should work anywhere sensible within the main Anvil window.

Dragging a supported file onto Anvil should immediately begin opening it rather than requiring another confirmation dialog.

## Data grid

The main screen should contain a high-performance virtualized grid.

Include:

- sticky/fixed column headers
- row numbers
- horizontal scrolling
- vertical scrolling
- resizable columns
- sensible automatic column widths
- selectable cells
- selectable rows
- keyboard navigation
- copying cells/rows to clipboard
- column highlighting
- clear rendering of null/empty values

The UI must not instantiate a permanent visual component for every row in a large file.

## Search

Provide extremely fast search.

Eventually support:

- global text search
- search within a selected column
- next/previous match
- match counts where feasible
- case sensitivity toggle
- exact vs contains matching

Search must not freeze the UI.

## Filtering

Design a useful filtering system.

Start with simple column predicates such as:

- equals
- not equals
- contains
- does not contain
- starts with
- ends with
- empty
- non-empty
- numeric comparisons

The filtering architecture should allow more advanced expressions later.

A filtered result should refer back to original row positions rather than unnecessarily duplicating entire records.

## Sorting

Allow sorting by column.

Support at minimum:

- ascending
- descending
- clearing sort

Use inferred column types when possible so numeric values do not sort lexicographically.

Do not block the GPUI event loop while sorting large datasets.

## Column inspection

This is particularly important for my work.

Selecting a column should allow Anvil to show an inspection panel containing useful information such as:

- column name
- inferred data type
- total row count
- non-empty count
- empty/null count
- unique count
- duplicate count
- approximate or exact cardinality, depending on cost
- minimum/maximum for numeric/date-like fields
- representative sample values
- most frequent values
- longest value
- shortest value

For very large files, statistics may initially be approximate or calculated asynchronously.

Clearly distinguish approximate statistics from exact statistics.

## Data-type inference

Infer useful semantic types such as:

- integer
- float
- boolean
- text
- date
- datetime
- UUID-like identifier
- email-like value
- URL-like value
- mostly-null column

Keep inference probabilistic/heuristic rather than pretending ambiguous data has a definitive schema.

## File overview

Create an overview or inspector area showing things like:

- filename
- archive filename, when applicable
- internal archive entry filename, when applicable
- file size
- compressed/uncompressed size when applicable
- number of rows
- number of columns
- delimiter
- detected encoding if available
- headers detected
- parse warnings
- malformed rows
- processing/indexing status

## Diagnostics

Anvil is a troubleshooting tool, so malformed data should be surfaced rather than silently hidden.

Track things such as:

- inconsistent column counts
- invalid quoting
- encoding issues
- unusually long records
- duplicate headers
- empty headers
- parsing failures
- archive corruption
- suspicious archive entries
- decompression failures

Provide enough context to locate the offending row or archive entry.

## Architecture

Before implementing everything, inspect the repository and propose a concrete architecture.

I expect something roughly resembling:

- `app`
  - GPUI application/window lifecycle
- `ui`
  - grid
  - toolbar
  - inspector
  - status bar
  - archive file picker
  - dialogs
- `source`
  - logical data-source abstraction
  - normal files
  - ZIP archives
  - archive entries
  - temporary resolved files
- `document`
  - currently opened dataset
  - file metadata
  - row addressing
- `parser`
  - delimiter handling
  - CSV parsing
  - record indexing
- `query`
  - search
  - filters
  - sorting
- `schema`
  - column metadata
  - type inference
  - statistics
- `tasks`
  - background operations
  - cancellation
  - progress
- `platform`
  - clipboard/file-dialog/drag-drop/native integration where needed

Do not mechanically follow that layout if the repository or GPUI idioms suggest a better design.

Explain major deviations.

## Data representation

Spend real engineering effort deciding how rows should be represented.

Consider tradeoffs among approaches like:

- byte offsets into the original file
- memory-mapped source files
- indexed record boundaries
- lazy decoding
- cached visible rows
- parsed column values
- columnar representations for derived operations

Do not duplicate a multi-gigabyte file in memory just to display it.

The architecture should allow the visible grid to retrieve arbitrary rows efficiently without requiring every field to permanently exist as an allocated `String`.

For compressed data, establish a clean transition from archive entry to the random-access representation used by the rest of Anvil.

## Concurrency

GPUI responsiveness is critical.

Create a deliberate model for:

- file loading
- archive inspection
- decompression
- indexing
- parsing
- statistics
- search
- filtering
- sorting

The UI should remain interactive while those operations execute.

Where useful, expose:

- progress
- cancellation
- generation IDs/task IDs so obsolete work cannot overwrite newer state

If the user drops another file while a large archive is still decompressing/indexing, obsolete work should be cancellable or safely ignored.

Avoid concurrency designs that make state ownership impossible to reason about.

## UX direction

The visual design should feel like a modern native technical tool.

Think:

- restrained
- information-dense without being cluttered
- fast
- keyboard friendly
- dark/light mode compatible
- appropriate for someone staring at datasets for hours

Do not spend significant effort on decorative visual design before the grid interaction and performance are solid.

A rough layout could be:

- top toolbar
  - filename
  - archive/entry indicator when relevant
  - open
  - search
  - filtering controls
- center
  - virtualized table
- optional right-side inspector
  - selected column/cell details
- bottom status bar
  - rows
  - columns
  - filtered row count
  - file size
  - processing state

If a ZIP contains multiple data files, provide an unobtrusive archive browser or file switcher rather than forcing the user through repeated modal dialogs.

## Keyboard-first interaction

Design useful shortcuts.

Examples:

- Cmd/Ctrl+O — open file
- Cmd/Ctrl+F — search
- Cmd/Ctrl+C — copy selection
- Escape — close search/filter overlays
- arrow keys — navigate grid
- Page Up/Page Down
- Home/End equivalents
- Cmd/Ctrl+G — go to row

Keep shortcuts platform-appropriate.

## Scope management

Do not attempt to build the entire final vision before establishing the core architecture.

Work in vertical slices.

### Milestone 1

Get a real GPUI application running with:

- native window
- normal file opening
- drag/drop
- direct ZIP opening
- automatic discovery of supported files inside ZIPs
- automatic opening when a ZIP contains one supported dataset
- a simple internal archive-entry selector when multiple datasets exist
- transparent decompression/extraction of the selected entry
- CSV parsing
- column headers
- virtualized row rendering
- vertical/horizontal scrolling
- status information
- a large generated test CSV
- a ZIP containing that large generated CSV

Prove that the architecture can display a large dataset responsively whether the source is a direct CSV file or a CSV contained inside a ZIP archive.

### Milestone 2

Add:

- selection
- keyboard navigation
- copying
- search
- basic filtering

### Milestone 3

Add:

- sorting
- type inference
- column inspector
- statistics

### Milestone 4

Add:

- malformed-record diagnostics
- richer filters
- archive diagnostics
- quality-of-life UX
- performance profiling and optimization

Do not treat later milestones as mandatory before Milestone 1 is verified.

## Testing

Create automated tests for data logic independent of GPUI wherever possible.

Test cases should include:

- normal CSV
- TSV
- quoted delimiters
- quoted newlines
- escaped quotes
- missing values
- empty columns
- duplicate headers
- inconsistent row lengths
- extremely wide files
- extremely long cells
- files without headers
- malformed input
- large generated datasets
- ZIP with one CSV
- ZIP with multiple CSV files
- ZIP containing nested directories
- ZIP containing irrelevant files
- ZIP containing `__MACOSX`
- corrupted ZIP
- ZIP with no supported data files
- suspicious archive paths such as `../file.csv`
- large compressed CSV
- archive entries with duplicate names

Add benchmarks around expensive operations when useful.

Generate synthetic datasets and archives for performance testing rather than relying only on tiny fixtures.

## Engineering rules

Before changing code:

1. Inspect the repository.
2. Inspect existing Cargo configuration and dependencies.
3. Understand the current GPUI setup.
4. Identify what already exists.
5. Avoid replacing working architecture unnecessarily.

When making important choices:

- state the alternatives you considered
- choose one
- explain the tradeoff briefly
- implement the decision

Do not invent APIs for dependencies. Inspect their actual available APIs/version before using them.

Avoid adding dependencies merely for convenience when a small implementation is straightforward, but also do not reimplement complex, well-solved parsing or archive primitives without a reason.

Keep modules cohesive.

Avoid giant files and giant state objects.

Use strong Rust types for important concepts such as row indexes, column indexes, task generations, document identity, archive identity, and archive-entry identity where doing so prevents mistakes.

## Verification loop

For each meaningful implementation step:

1. implement it
2. compile it
3. fix compiler errors
4. run relevant tests
5. launch/run the application where possible
6. verify the actual behavior
7. record what remains

Do not stop at “the files were created.”

A successful milestone means the code builds and the behavior has been exercised.

If something cannot be verified, clearly state exactly what remains unverified.

## JEV / agent-routing behavior

Use the strengths of my JEV-routed setup rather than treating every task as the same kind of problem.

Keep work decomposed into clear phases such as:

- **inspect** — understand repository/API/current implementation
- **implement** — make the selected change
- **verify** — compile, test, benchmark, or exercise behavior
- **answer** — summarize decisions/results when no tools are needed

Give decisions a concrete shape.

When there are multiple plausible implementations, define the finite candidate options, evaluate the relevant tradeoffs, select one, and proceed.

Do not allow model routing to invent capabilities, providers, commands, or permissions. The host environment remains authoritative.

## First task

Begin by inspecting the repository and GPUI setup.

Then:

1. summarize the existing project state
2. propose the architecture for Anvil
3. identify the libraries you recommend for the initial implementation and why
4. describe the intended large-file data representation and row-indexing strategy
5. describe how ZIP files will be represented and resolved into browsable datasets
6. identify the primary performance risks
7. establish Milestone 1
8. implement Milestone 1 rather than stopping after the design discussion
9. compile and test it
10. generate a large synthetic CSV
11. generate a ZIP containing that CSV plus some irrelevant files
12. test opening both the direct CSV and ZIP versions through drag/drop
13. report what is working, what you measured, and what should come next

The immediate goal is to get from an empty/early repository to a **real, running Anvil application that can open and smoothly inspect a large CSV, including when that CSV is delivered inside a ZIP archive**.

Favor a small, solid, measurable core over a broad but shallow implementation.

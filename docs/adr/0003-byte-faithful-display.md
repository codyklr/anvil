# ADR 0003: Byte-faithful display

## Status

Accepted. Applied to the grid, the clipboard and the column inspector.
Some comparison and classification semantics were deliberately left
alone and are tracked in `ROADMAP.md`.

## Context

Anvil exists to show what is actually inside a file. The product owner
stated the philosophy explicitly: no data should ever be transformed.
Data must be visible exactly as it exists in the file - never stripping
leading or trailing whitespace or zeroes, never truncating, never
reformatting.

An audit against the code found three places that contradicted it:

1. `schema::stats` (the column inspector) stored
   `cell.as_str().trim()` for `min`, `max`, `longest`, `shortest` and
   `samples`. A value of `" 42 "` was displayed as `42`, and
   `distinct` / `frequent` merged values that differed only in padding.
2. `ui::grid` rendered `str::from_utf8(bytes).unwrap_or("")`, so a cell
   containing invalid UTF-8 was displayed as the empty marker `∅`. The
   data was hidden rather than shown.
3. `Cell::into_string` used `String::from_utf8_lossy`, so copying such a
   cell produced U+FFFD replacement characters.

## Decision

Display and copy are byte-faithful; ordering and classification stay
pragmatic and are documented as such.

- The column inspector reports the exact value for `min`, `max`,
  `longest`, `shortest`, `samples` and `frequent`, and counts values
  that differ only in padding as distinct values, because they are
  different values.
- Cells that are not valid UTF-8 are rendered and copied through
  `parser::exact_string`: valid UTF-8 is returned borrowed and
  unchanged, and otherwise a literal backslash becomes `\\` while each
  invalid byte becomes `\xNN` (lowercase hex). Doubling backslashes
  across the whole cell is what makes the mapping reversible, so a real
  backslash can never be confused with the start of an escape.
- `Cell::as_str` and `Cell::into_string` keep their lossy behaviour for
  internal callers that need a `&str` (byte search, encoding counts),
  but neither may be used for display or for the clipboard.
- Every display path uses it: the grid, the clipboard, the `--probe`
  sample rows, and the column headers (`ParseMeta.headers` is built with
  `exact_string` instead of `from_utf8_lossy`).
- Grid ellipsis (GPUI's `truncate()`) is a layout concern only: the full
  value stays in the document, the inspector and the clipboard.

## Alternatives considered

- **U+FFFD for invalid bytes.** Conventional, but lossy, and
  indistinguishable from a genuine U+FFFD already present in the file.
- **Decoding non-UTF-8 as Latin-1 / CP1252.** More readable, but it
  asserts an encoding Anvil has not verified; a wrong guess would be its
  own transformation.
- **Escaping only the invalid bytes, leaving backslashes alone.** This is
  ambiguous: a valid run containing the literal text `\xff` could not be
  told apart from an escaped byte.
- **Removing grid ellipsis entirely.** A grid has to fit its viewport.
  Horizontal scrolling already exposes wide values and copy is exact, so
  elision is a display affordance rather than data loss.

## Consequences

- Escaped cells can be longer than the bytes they represent, but the
  escaping only runs when a cell actually contains an invalid byte, so
  the common all-valid-UTF-8 path borrows and stays allocation-free
  apart from the existing text copy.
- The inspector can now report a higher `distinct` count on padded data,
  and its `frequent` list shows the exact spellings.
- Follow-up decisions (Iteration 8): text `min`/`max` ordering and text
  sorting now compare exact bytes with a deterministic tiebreak, and a
  whitespace-only cell is a value, not empty. The filter `empty`
  operator already used `bytes.is_empty()`, so the inspector was
  changed to agree with it rather than the other way round.
- Tests lock both behaviours: the escaping in `tests/core.rs`
  (`invalid_utf8_cells_are_shown_exactly`,
  `exact_string_escapes_only_when_it_has_to`) and the padded inspector
  values in `schema::stats`
  (`padded_values_are_reported_exactly`,
  `whitespace_only_cells_still_count_as_empty`).

# ADR 0002: Optional native GLiNER Ask interpreter

## Status

Accepted for an opt-in, single-condition implementation; see the agent log
for verification results and unresolved quality limits.

## Context

Anvil already has a rules-first Ask bar and an optional Candle GGUF
backend. The user requested GLiNER with native Rust inference and a small
installation burden. A Python helper would require another runtime and
PyTorch. The existing filter AST and preview/accept flow must remain the
execution boundary.

## Decision

Add `nl-gliner`, off by default, using published `gliner-rs` 0.2.3 and
Candle 0.11 on CPU. Select `fastino/gliner2.5-small-v1` at revision
`7132dc4561c3f94563c6147e75ffa8ef34c4964a` (74M parameters, F32).
Reuse the Ask interpreter seam. If both model features are enabled,
GLiNER takes precedence; the generative backend remains available alone.

Classify the target column and operator using closed label lists and
extract a literal value with source offsets. Require confidence >= 0.65,
an existing unique column, a known operator, exactly one value when needed,
and an exact character-span match. Ordering also requires a numeric column
and a finite number. Render and reparse the result to verify that the filter
bar preserves the same AST. No generated code or SQL is executed.

Limit input bytes, column count, schema bytes, and actual combined tokens;
never silently truncate the question or column list. Reject common compound,
aggregate, sorting, and relative-date phrases. These guards are deliberately
conservative, not a proof of complete semantic coverage. Preview remains
mandatory, since confidence does not guarantee a correct interpretation.

A mutex retains one model for the interpreter lifetime. Inference executes
on GPUI's background executor. A concurrent request gets a busy error rather
than queuing additional work. Editing, closing, changing files, and header
reindexing invalidate old previews/results. In-flight CPU work can finish,
but its obsolete result cannot replace the current preview.

Model setup is explicit (`--setup-ask-model`), uses the pinned revision,
bounded downloads, temporary files, and atomic publication. Inference is
network-free. No Python or external runtime is installed. Custom checkpoint
paths are supported; upstream model format compatibility still applies.

## Alternatives considered

- Python GLiNER: rejected because of the user's native-runtime constraint.
- ONNX Rust bindings: rejected because they add a separately distributed
  native runtime rather than sharing Candle.
- Port or vendor the model ourselves: avoids upstream CLI dependencies but
  transfers model maintenance and parity testing to Anvil. Defer until the
  upstream dependency overhead justifies maintaining a fork.
- Existing generative model: retained as an alternative; it has larger
  weights and an autoregressive generation loop.

## Consequences

The default build does not acquire the model stack. The optional feature
still adds substantial Rust inference dependencies: native does not mean
zero dependencies. `gliner-rs` currently includes clap and rustyline even
for library users, and uses tokenizers 0.22 (the older backend uses 0.23).
The pinned published crate avoids a local fork or moving Git dependency.

The model weights are roughly 296 MB, with additional tokenizer data and
runtime memory. Setup is revision-pinned, not independently checksum-verified.
GUI download progress, hard cancellation, calibrated task-specific accuracy,
compound model parsing, and broader platform verification remain follow-ups.

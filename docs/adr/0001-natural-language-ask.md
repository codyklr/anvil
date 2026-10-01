# ADR 0001: Natural-language Ask bar (rules first, optional local model)

## Status

Accepted. Rules and the Ask UI are implemented; model verification and
distribution status are recorded below and in ADR 0002.

## Context

Users think in questions ("US orders over 100", "failed payments this
month") but the filter bar speaks a small structured grammar. Anvil
already produces a typed filter AST: `FilterExpr` (predicates combined
with `And` / `Or`), each predicate a `(column, FilterOp, value)`. It also
already exposes the context needed to disambiguate language: column names
from `ParseMeta.headers` (plus generated `colN` labels), column types
from `schema::infer`, and real sampled values from `sample_distinct`.

Two broad approaches exist:

- **Rules-based semantic parsing.** Deterministic, offline,
  dependency-free, fast, and easy to test. Handles common phrasing well
  and can report exactly what it could not parse. Fails on free-form
  language, coreference, and date expressions such as "last quarter".
- **A language model.** Handles arbitrary phrasing, but a local model is
  a large optional dependency (model download, cross-platform native
  build, binary size) and a cloud model requires network, keys, and
  sending dataset contents off-machine.

Anvil is native, offline-first, fast, and inspection-first. It must not
acquire a hard dependency that compromises any of those, and it must
never apply a query it is not confident about.

## Decision

Ship an opt-in **Ask** bar bound to `Cmd/Ctrl+Shift+L` that builds a
`FilterExpr` from a phrase using a **hybrid, rules-first** design.

1. **Tier 1 (always available).** A GPUI-free `anvil_core::query::nl`
   module parses the phrase with deterministic rules.
2. **Tier 2 (opt-in).** If, and only if, Tier 1 does **not cleanly
   resolve** the phrase and an optional local-model backend is enabled,
   escalate to that backend.
3. The Ask bar **previews** the resulting expression; the user accepts or
   edits it in the normal filter bar. The result is **never** evaluated
   directly, and never silently applied.

### The clean-resolution contract

Tier 2 is invoked only when Tier 1 fails **any** of these. If all hold,
Tier 1's answer is used and no model is consulted.

- At least one predicate was produced.
- Every non-stopword token is consumed; there are no unmatched spans.
- Every column mention resolved to a real column (header, `colN`, or
  alias) with no ambiguity.
- Every value bound to a real sampled value (text columns) or a valid
  literal (numeric / boolean).
- Every predicate has high confidence (the column, operator, and value
  all matched unambiguously).

If Tier 1 fails and Tier 2 is unavailable or disabled, the Ask bar shows
the partial parse plus the unmatched spans so the user can fix it by
hand. It never guesses silently.

### Tier 2 backend interface

A `NlBackend` trait (`fn parse(&self, phrase, schema) -> Result<FilterExpr, NlError>`)
with two implementations: `RuleBackend` (Tier 1) and an optional
`ModelBackend`. Requirements:

- Feature-flagged (`nl-model`), **off by default**; no new dependency in
  the default build.
- Runs fully locally. No network calls. The model file is supplied by the
  user (a configured path); it is never downloaded automatically.
- The model must emit the existing grammar (or JSON matching
  `FilterExpr`), and its output is **always re-parsed and schema-validated**
  by `parse_expr` before it reaches the preview. Raw model output is
  never trusted.
- If the feature is off or the model is missing/unloadable, the Ask bar
  still works with Tier 1 alone.

## Alternatives considered

- **Rules only.** Simplest and dependency-free, but leaves free-form
  phrasing unhandled. Rejected as the final state; kept as the default
  tier.
- **Model only.** Better free-form coverage, but the default experience
  would depend on a heavy artifact or a network service, and would send
  column names and values off-machine. Rejected.
- **Cloud API for Tier 2.** Best raw quality, but violates offline and
  privacy expectations and complicates cross-platform/CI. Rejected.
- **Auto-applying the parse.** Faster, but a wrong filter that silently
  hides rows is worse than no feature. Rejected in favour of preview and
  accept.

## Consequences

- The filter AST is the single output contract; evaluation, sorting,
  highlighting, and suggestions are unchanged.
- Tier 1 is pure `&str -> Result<FilterExpr, Unparsed>` and is
  exhaustively unit-testable, matching the existing `query::filter` and
  `query::suggest` style.
- `NOT` and a column-alias map become prerequisites: natural language
  wants "not"/"except", and aliases make column matching forgiving.
  `FilterExpr` currently has no negation.
- The optional model adds a feature flag and a lazily loaded backend;
  the default binary stays lean and offline.
- Cost is one-shot (on submit), so it does not affect per-frame work.

## Refinement (2026-11-15): Tier 1.5 is the default; the model is opt-in

Two user constraints reshaped the original plan: the user must not have
to pre-install anything, and it must stay lightweight on macOS, Windows,
and Linux.

The original decision left the engine open and assumed an external or
pre-installed model. That is rejected by the no-pre-install constraint,
and a bundled or auto-downloaded model with an embedded runtime fights
the lightweight ethos (a heavy dependency tree plus a large first-run
download).

Decision:

- The default Tier 2 is **Tier 1.5**: extended deterministic rules
  (filler normalization, fuzzy column matching, `between`, more operator
  synonyms, and later relative dates and a synonym file). It needs no
  install, no download, and no new dependency, works on every platform,
  and emits the same `FilterExpr` that `parse_expr` validates.
- The true model path stays available as an **opt-in embedded backend**
  behind the `nl-model` cargo feature, off by default. Planned stack:
  `candle-core` + `candle-transformers` + `tokenizers` + `hf-hub`, one
  pinned quantized instruct model (for example SmolLM2-360M-Instruct Q8)
  downloaded once after explicit consent into the platform cache
  directory, checksum-pinned and offline afterwards. None of it is in the
  default build.
- A subprocess `ModelBackend` remains as a power-user override. It is
  never required and never the default.

The escalation contract is unchanged, now with three tiers: Tier 1
rules, then Tier 1.5, then (only if enabled and present) the embedded
model. Every non-rules result is re-validated by `parse_expr`.

## Implementation status

- Tier 1 and Tier 1.5 are implemented in `anvil_core::query::nl` and ship
  in the default build.
- The embedded backend (`src/nlmodel.rs`) is implemented behind the
  off-by-default `nl-model` feature using `candle-core`,
  `candle-transformers`, `tokenizers`, and `ureq`. The default model is
  Qwen2.5-Coder-1.5B-Instruct Q4_K_M; the GGUF architecture is read from
  metadata so both Llama and Qwen2 families load. It is invoked from
  the Ask bar with `Ctrl/Cmd+Enter` when the rules do not cleanly
  resolve a phrase, and its candidate is always validated by
  `validate_candidate`.
- Downloads use `ureq` directly rather than `hf-hub`: hf-hub 0.3 read
  the Hub's redirect `Location` header and re-issued it unresolved,
  which fails on a relative redirect with "relative URL without base".
  `ureq` resolves redirects against the request URL.
- Remaining follow-up: the download is currently implicit (it starts on
  the first Ctrl/Cmd+Enter) and has no size dialog, progress bar, or
  cancel. The model file is also not checksum-pinned beyond what
  `hf-hub` provides.
- The external subprocess `ModelBackend` remains available as a
  power-user override and is not required.

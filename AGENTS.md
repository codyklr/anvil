# Anvil — Agent Operating Manual

> Any agent working on Anvil should begin by understanding the current
> repository state before modifying it. The repository, tests,
> roadmap, architecture documentation, and recent agent log are the
> source of truth.

This file describes how coding agents are expected to work in this
repository. It exists so that development is an iterative,
self-documenting agentic cycle rather than a series of disconnected
coding sessions.

---

## Required Reading Order

Every agent should read, in order, before writing any code:

1. `/AGENTS.md` — this file.
2. `/VISION.md` — the durable product intent.
3. `/ROADMAP.md` — what has been done and what is next.
4. `/ARCHITECTURE.md` — how the code is currently organised.
5. The most recent entries in `/docs/agent-log.md`.
6. The relevant source code and tests for the slice of work you are
   about to touch.

The original prompt at `/docs/prompts/initial-anvil-prompt.md` is
historical context. Consult it when useful, but it is **not** more
authoritative than the current architecture or the current code.

---

## Source-of-Truth Priority

When information conflicts, use this priority:

1. **Working repository behaviour and tests.** `cargo build`,
   `cargo test`, `cargo run --probe` are the ground truth.
2. **Current source code.** `src/**` over any document.
3. `ARCHITECTURE.md`.
4. `ROADMAP.md`.
5. `VISION.md`.
6. Historical prompts and older agent-log entries.

If documentation disagrees with code, investigate why and update the
stale documentation in the same change. Do not silently force the
code back toward stale documentation.

---

## Agent Startup Procedure

Before implementing anything, every agent must:

1. Inspect the current git/repository state (`git status`,
   `git log --oneline`).
2. Read the required project docs (`AGENTS.md`, `VISION.md`,
   `ROADMAP.md`, `ARCHITECTURE.md`, recent `docs/agent-log.md`).
3. Determine the active milestone from `ROADMAP.md`.
4. Inspect the relevant existing implementation in `src/`.
5. Identify already-completed work.
6. Identify any partially-completed work (search for `TODO`,
   `unimplemented!`, `todo!`, half-finished modules).
7. Check recent agent-log entries for known issues and open loops.
8. Determine the **smallest** useful next implementation slice.
9. Understand its acceptance criteria (from `ROADMAP.md`).
10. Avoid reimplementing something that already exists.

If the roadmap does not accurately describe reality, **reconcile the
roadmap first** before proceeding. The roadmap is a living plan;
outdated milestones are themselves a bug.

---

## Iterative Development Cycle

Agents should work in small, verifiable iterations. Use this cycle:

### 1. Orient

- Where does the project currently stand?
- Which milestone is active?
- Which architecture sections apply?
- What did the previous agent leave behind?
- What known issues are recorded in the agent log?

### 2. Select

Choose one coherent next slice of work. Prefer completing a narrow
**vertical capability** (e.g. "cell selection with arrow-key
navigation and copy") over touching many unrelated areas at once
(e.g. "refactor the layout system and also add filter"). If a
roadmap task reveals a prerequisite, record that prerequisite and
handle it deliberately — do not silently let it block other work.

### 3. Inspect

Read the relevant implementation and dependency APIs **before**
modifying code. GPUI is fast-moving; do not invent APIs from memory.
Run `cargo doc --open` for unfamiliar crates, or read the relevant
source.

### 4. Implement

Make the smallest architecturally sound change that completes the
selected slice. Honour the architectural invariants in
`ARCHITECTURE.md`.

### 5. Verify

At minimum, as relevant:

- `cargo fmt`
- `cargo build --release`
- `cargo test --release`
- `cargo clippy --release` (when available)
- Add tests for new behaviour
- `cargo run --release --bin anvil --probe <fixture>` for
  large-file sanity
- Launch the actual app (`cargo run --release --bin anvil`) and
  exercise the UI
- Use the synthetic large dataset under `fixtures/`
- Verify archive behaviour against `fixtures/multi.zip`
- Benchmark / profile when performance is the purpose of the change

**Do not consider code complete solely because it compiles.**

### 6. Reconcile

After verification:

- Tick the relevant roadmap checkboxes.
- Update `ARCHITECTURE.md` if architecture changed.
- Add any discovered technical debt to the
  "Current Limitations / Technical Debt" section of
  `ARCHITECTURE.md`.
- Do not mark partially working features complete.

### 7. Handoff

Commit the iteration on its conventional branch, open and merge the
pull request (see Version Control Workflow), then append a concise
entry to `/docs/agent-log.md` and identify the logical next task.

---

## Version Control Workflow

Every coherent change set lands on its own branch and is merged to
`master` through a pull request. Do not push directly to `master`.

- **Branch name:** a Conventional Commits type prefix plus a short
  kebab-case summary, e.g. `fix/linux-build`,
  `feat/non-contiguous-select`, `refactor/view-split`,
  `docs/agent-workflow`. Common types: `feat`, `fix`, `refactor`,
  `perf`, `docs`, `test`, `chore`.
- **Commits:** Conventional Commits — `<type>(<scope>): <imperative
  subject>` — with a body explaining *why*. One roadmap iteration is
  one commit; do not combine unrelated work.
- **Pull request:** open a PR into the default branch (`master`) and
  state what changed, how it was verified, and what was **not**
  verified. Reference the roadmap item or issue when there is one.
- **Merge:** once CI is green and the Definition of Done is met, merge
  the PR (a merge commit preserves the per-iteration commits), delete
  the branch, and fast-forward local `master`.
- If no remote or credentials are available, say so in the handoff
  rather than bypassing the PR and pushing to `master`.

---

## Scope Discipline

Agents must **not**:

- Jump ahead simply because a later feature looks more interesting.
  Milestone 1 must remain solid before Milestone 2 work lands.
- Rewrite working architecture without a concrete, recorded reason.
- Combine unrelated refactors with feature work unnecessarily.
  Land refactors in their own iteration.
- Convert Anvil into a generic spreadsheet application.
- Introduce browser / webview stacks.
- Transform data on its way to the screen. Values must be displayed
  exactly as they exist in the file: never trim, reformat, round,
  re-encode or truncate a value. See the Data Fidelity section of
  `VISION.md` and `docs/adr/0003-byte-faithful-display.md`.
- Perform large scans on the UI thread.
- Sacrifice large-file behaviour for implementation convenience.
- Scatter persistent program state (downloaded models, caches, future
  configuration) into the platform cache/config directories. Keep it
  beside the executable so the install stays self-contained; see the
  self-contained principle in `VISION.md` and `src/paths.rs`.
- Add dependencies without weighing the maintenance cost.
- Use `unsafe` outside the existing `MmapSource::open` site without
  an explicit justification comment (and, for durable changes, an
  ADR).

If a roadmap task reveals a prerequisite that has not been planned,
record that prerequisite and either (a) handle it in this iteration
or (b) add it explicitly to `ROADMAP.md` as a new task and pick it up
in a later iteration.

---

## UI Design Conventions

Read [docs/ui-design.md](docs/ui-design.md) before changing presentation.
The GPUI Kit redesign is an investigation workspace, not a spreadsheet
rewrite. Preserve these boundaries:

- Use `gpui-kit = "=0.7.0"` as the single direct UI dependency and import
  framework/component/assets APIs through `gpui_kit`. The lockfile uses
  `gpui-pre` 0.3.7; do not mix in a second GPUI runtime or invent APIs.
- Prefer Kit Buttons, menus, dialogs, native-aware TitleBar and resizable
  primitives for shell controls. Keep platform window behavior intact.
- Keep raw color definitions in `src/ui/theme.rs`; consume the active Kit
  theme or semantic `Palette` snapshots, never inline renderer hex colors.
  Review both Anvil Slate (dark) and Anvil Warm (light).
- Keep one right-hand auxiliary surface at a time (Column, Diagnostics,
  or Cell). Pane layout and grid viewport calculations must share the
  geometry in `view/workspace.rs`; never infer widths from stale bounds.
- Retain the custom virtualized grid and its physical metrics: 22 px rows,
  12 px monospace data text using Kit's mono family. Preserve the bounded
  text cache, byte fidelity, row-reference queries and asynchronous scans.
- Query bars intentionally retain `TextEdit` and shared `ui/input.rs`
  rendering/hit-testing. A styling migration does not imply native IME or
  composition support; any input-model replacement is a separate slice.
- Distinguish implementation from evidence. Inspect the native window at
  normal and 900 x 480 minimum sizes, in both appearances, and record
  actual test/probe/manual results before claiming visual acceptance.

---

## Performance Discipline

Anvil is fundamentally a high-performance data viewer. Performance
regressions should be treated as bugs. Therefore:

- Avoid unnecessary allocation.
- Avoid full-file duplication.
- Avoid owned strings for every cell unless explicitly justified.
- Preserve virtualization (no per-row persistent UI nodes).
- Preserve bounded-memory design.
- Run expensive work asynchronously.
- Make cancellation / obsolescence behaviour explicit.
- Benchmark before introducing complicated optimisation.
- Record useful performance findings in the agent log.

When making a performance change, measure **before** and **after** on
the same machine with the same fixture, and record both numbers.

---

## Testing Discipline

Every meaningful piece of non-UI logic should be testable
independently where practical.

When fixing a bug:

1. Reproduce it (write a failing test if you can).
2. Add a regression test when reasonable.
3. Fix it.
4. Verify the test passes.

Important edge cases to keep in mind (covered today and to be kept
covered as the codebase grows):

- Malformed CSV (unterminated quotes, wrong row widths, etc.).
- Quoted newlines and escaped quotes.
- Huge cells (multi-MB values).
- Inconsistent rows.
- Missing headers / headerless files.
- Files with no trailing newline.
- ZIPs with single, multiple, or no usable data files.
- ZIPs with nested directory paths, `__MACOSX`, `.DS_Store`,
  hidden files.
- Invalid / corrupted ZIPs.
- Path traversal (`..`) in archive entry names.
- Duplicate archive entry names.
- Large compressed CSVs (50k+ rows inside a ZIP).

---

## Documentation Discipline

Update documentation as part of the same change when reality
changes.

### `ROADMAP.md`

Update when:

- Tasks are completed.
- Acceptance criteria change.
- New dependencies are discovered.
- A milestone needs refinement.

### `ARCHITECTURE.md`

Update when:

- Module boundaries change.
- State ownership changes.
- Data representation changes.
- Concurrency changes.
- Indexing changes.
- Archive handling changes.
- Important dependencies change.

### `VISION.md`

Update **only** when the actual product direction intentionally
changes. Do not modify the vision merely because implementation
details changed.

### `docs/agent-log.md`

Every meaningful implementation iteration should add an entry. Use
the template in the file's existing entries.

### `docs/adr/` (planned)

For unusually significant or hard-to-reverse decisions, add an ADR:

```
/docs/adr/NNNN-<slug>.md
```

numbered sequentially. Each ADR records:

- **Context** — what was the situation?
- **Decision** — what was chosen?
- **Alternatives considered** — what else was on the table?
- **Consequences** — what follows?

Do not create ADRs for trivial implementation choices. The
generation counter for cancellation, the mmap-based data
representation, and the per-row byte-offset index are good
candidates for ADRs but were not yet recorded as such at Milestone 1.

---

## Definition of Done

A task is **done** only when **all** of the following hold:

- Implementation exists in the repository.
- `cargo build --release` succeeds.
- Relevant tests pass (`cargo test --release`).
- Behaviour has been exercised — by a test, by `--probe`, or by
  manually launching the GUI.
- Errors discovered during verification have been handled.
- Documentation reflects the result (`ROADMAP.md` ticked,
  `ARCHITECTURE.md` updated if applicable).
- An agent-log handoff entry has been written.
- The change is on a conventional branch and has been merged to
  `master` through a pull request.

If any of the above cannot be verified, **state that explicitly**
rather than pretending completion. "Tests pass; manual UI exercise
deferred" is fine. "It compiles" alone is not.

---

## Handling Existing Work

- **Never** assume an unchecked roadmap item means no implementation
  exists. Inspect first.
- **Never** assume a checked item is correct. Read the code or run
  the tests.
- If code or tests show a roadmap item is actually incomplete,
  fix the roadmap.
- Agents should **preserve useful incomplete work** when practical
  rather than restarting from scratch.

---

## Architectural Decision Making

For significant architectural choices:

1. Identify realistic options.
2. Compare tradeoffs.
3. Choose one.
4. Implement it.
5. Record the decision — either in `ARCHITECTURE.md` (small) or in
   `/docs/adr/NNNN-<slug>.md` (large / hard to reverse).

A candidate for a small ADR-style note in `ARCHITECTURE.md`:

- Background executor + generation counter for cancellation
  (rationale, alternatives, consequences).

A candidate for a real `docs/adr/` entry:

- Memory-mapped representation for direct files (random row access
  - zero copy vs. tradeoffs like size and platform behaviour).
- Decompress-to-temp + mmap strategy for ZIP entries (vs. streaming
  into the indexer).

The next agent should consider creating at least one ADR during the
Milestone 2 work, since search/filter results will lock in the
"row-reference over copied record" invariant.

---

## End-of-Iteration Report

Every coding iteration should end with a short report containing:

- **Implemented** — what changed in this iteration.
- **Verified** — what was exercised (tests, probe, manual).
- **Tests added/changed** — list of new or modified tests.
- **Roadmap changes** — which checkboxes moved.
- **Architecture changes** — what (if anything) in
  `ARCHITECTURE.md` was updated.
- **Issues / technical debt discovered** — anything worth tracking.
- **Next recommended task** — the smallest useful next slice.

These reports are the body of each `docs/agent-log.md` entry. Keep
them short and factual; they are operational continuity, not a
verbose chain-of-thought.

---

## Open-source hygiene

- Format with `cargo fmt` before committing.
- Run `cargo clippy --release` if clippy is available in the
  toolchain (`rustup component add clippy`).
- Do not commit `/target`, generated fixtures, or `**/.DS_Store`.
  `.gitignore` covers `/target`, `.DS_Store`, and everything under
  `/fixtures/` except the hand-authored `fixtures/ask.csv`. The
  generated fixtures (`large.csv`, `archive.zip`, `multi.zip`,
  `malformed.csv`) are recreated with `gen_fixture` when they are
  needed and are never checked in.
- Use Conventional Commits: `<type>(<scope>): <imperative subject>`
  with a body that explains *why*, and land the work through a pull
  request (see Version Control Workflow). Never push directly to
  `master`.

---

## Quick reference — commands

```sh
# Build
cargo build --release

# Format
cargo fmt

# Lint
cargo clippy --release

# Tests. The count is whatever this command prints.
cargo test --release

# Throughput test (needs fixtures/large.csv)
cargo test --release --test core -- --ignored measure_large --nocapture

# Headless probe
cargo run --release --bin anvil --probe fixtures/large.csv

# Generate fixtures (1M rows by default)
cargo run --release --bin gen_fixture fixtures 1000000

# Launch the GUI
cargo run --release --bin anvil
```

When you finish, append your entry to `/docs/agent-log.md` and
update the relevant checkboxes in `/ROADMAP.md`.
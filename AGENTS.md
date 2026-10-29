# adtb

A new database engine with native sum types (algebraic data types) at
both the value level (a column whose type is a union) and the relation
level (an entity set partitioned into constructors, with exclusivity,
totality and foreign keys enforced by the engine).

Rust crate, built with `cargo`. The dev shell is devenv (`devenv.nix`),
entered through direnv.

## Agent entry point

This file holds the shared project instructions for coding agents. Read
it directly if your harness does not load `AGENTS.md` automatically.
Keep project guidance here; `CLAUDE.md` imports this file for Claude Code.
Repository workflows live in `.agents/skills/`. They are Markdown files
you can read and follow even without a skill loader.

## Version control

The repo is jj, colocated with git. Use `jj`, not `git`, for commits,
history and branches: `jj status`, `jj diff`, `jj describe -m`,
`jj new`, `jj log`. Commit messages are one subject line in
conventional form (`feat: ...`, `docs: ...`).

Preserve uncommitted work. Never stash or discard changes to get a clean
tree. Propose a commit subject before committing when a commit is requested.

## In-progress work

If `work/HANDOFF.md` exists, read it first: it records where the design
work stands and the next steps.

The handoff is local and may be absent or stale. Use the current tickets,
code and `jj status` to establish implementation status. Record anything
needed to resume work in repository files, with paths and commands that
another agent can use without access to your session or private memory.

To dispatch, coordinate or land agent tracks, read and follow
[the lead workflow](.agents/skills/lead/SKILL.md).

## Build and checks

Enter the development environment with `direnv allow` or `devenv shell`.
Run commands from the repository root unless a workflow says otherwise.
For a fresh clone, host prerequisites and optional editor setup, read
[development setup](docs/development.md).

- `cargo build --workspace`: build the workspace.
- `cargo clippy --workspace --all-targets -- -D warnings`: lint Rust.
- `cargo fmt --all --check`: check Rust formatting.
- `work/tools/suite [ARGS]`: run tests through the shared nextest lock.
- `check` (in the dev shell): run the full checks, including the campaign
  application, dependency policy and source-size limits.
- `apps/campaign/` is its own cargo workspace, excluded from the root
  one. Build and test it from that directory.

Use targeted checks while working and run the checks appropriate to the
change before reporting completion.

## Tickets

Tracked work lives in `work/issues/` and is managed with `tix`; the
format and priority scheme are in `work/issues/README.md`. Design
decisions are not tickets: they go through the design space. Check
`tix list --ready` before starting work, and run `tix check` after
editing a ticket. A ticket file `tix new` creates can be ignored by a
global gitignore and so stay untracked: run
`jj file track --include-ignored work/issues/STEM.md` and confirm it with
`jj file list work/issues/STEM.md`. The devenv shell puts `tix` on PATH with `-C work/issues` set. Outside
it, run it from the repository root as:

```
nix run github:tonyd33/tix -- -C work/issues check
```

## Tools

Scripts in `work/tools/`, run as `work/tools/NAME` from any workspace
root (devenv also puts each on PATH as `NAME`):

- `suite [ARGS]`: the full nextest run, one at a time across the
  machine; output in `target/suite.txt`, prints failures and the summary.
- `conf PATH...`: run conformance files or directories on the
  reference and the engine; prints each difference as `FILE:LINE`.
- `adtb-probe --schema FILE [--data FILE] [--explain] TEXT`: check a
  text and run it on the reference and the engine side by side.
- `codemap [CRATE [MODULE]]`, `codemap --tests [CRATE]`: modules, pub
  items with line numbers, test binaries with their test counts.
- `house-check`: the added lines of `jj diff` that break a comment rule.
- `quality-metrics.sh [REVSET]` (`quality-metrics` in the dev shell):
  review metrics for the changes in REVSET, by default `reviewed..@`.
- `size-check.sh` (`size-check` in the dev shell): fail when a non-test
  source passes the line limit or the allowlist in `size-allowlist` is
  stale.
- `land WS [SUBJECT]`: land workspace WS on `main` and move the main
  working copy onto it; refuses while the main working copy has changes.
- `workspace-gc [--clean | --guard]`: every workspace's `target/` size;
  clean the idle ones; `--guard` fails under 30 GB free.

## Knowledge base

Facts about other databases, formats, languages and papers live in
`work/kb/`, one file per subject, each fact marked `[V date]`, `[S date]`
or `[U]` with its source. Format and rules in `work/kb/README.md`.
Before any web search or fetch, search `work/kb/`; after fetching,
append what was learned. Research agents get the same instruction.

## Comments and reviews

- Default to no comments. Keep comments for non-obvious invariants.
- Stop when the fact is stated; omit appended rationale and conversational
  history. No em dashes, external citations, rule IDs or Markdown bold in
  comments.
- Write doc comments in the imperative, or as "Returns ...". State real
  contracts as labelled preconditions and postconditions.
- Preserve existing `TODO`, `FIXME`, `IMPROVE` and `NOTE` comments through
  refactors. Do not invent speculative notes on the maintainer's behalf.
- An `#[expect]` reason states what would have to change to remove it.
- Before finishing, read the diff and run `work/tools/house-check`.

For a periodic quality review, read
[the quality-review workflow](.agents/skills/quality-review/SKILL.md).

## Specification

The spec is `docs/spec/`, one file per part; `README.md` there is the
index and assigns each topic to one file. Every part is normative and
states only behaviour users and clients observe, declaratively. Where
sources disagree: the conformance suite, then `semantics.md`, then the
other parts. A spec change that contradicts a ruling goes through the
design space first. Examples follow `syntax.md` and
`work/research/notes/notation.md`.

Contracts between subsystems live in `docs/design/`, one file per
boundary; a subsystem relies only on what its counterpart's design
file states. Behaviour inside one subsystem lives with its code. A
rule belongs to the lowest tier that can observe it. Treat any spec
change an implementation asks for as suspect: patch the spec only for
undecided or contradictory observable behaviour, never to promote an
internal choice into a promise.

Before writing or reviewing spec parts, read and follow
[the spec-revision workflow](.agents/skills/spec-revision/SKILL.md).

## Design space

Design decisions live in a quandary space at `work/research/`:
`questions/`, `notes/` (prior art under `notes/prior/`), `points/`
(rulings). Run `quandary` from inside `work/research/`;
`quandary check` after every edit,
`quandary status` for the frontier, `quandary show ID` for a node.
Rulings go through quandary commands, never by editing `points/`.

Before working in the space, read and follow
[the design-space workflow](.agents/skills/design-space/SKILL.md).

Before designing or implementing anything, read the rulings in
`current` (`quandary rulings`). Code follows them; a change that
contradicts a ruling is a design change and goes through the space
first.

The user owns design rulings. Start with `quandary status` and
`quandary show ID`; expand questions and options in `questions/`, and
keep evidence in `notes/`. Check `quandary --help` and the relevant
subcommand's help for command syntax. Before an edit spanning multiple
files, run `quandary lock 'agent: description'`; finish with
`quandary unlock`, which checks the space.

Explore unapproved choices in a proposed point extending `current`
(`quandary point new NAME current --proposed`). Before accepting an
option, run `quandary whatif accept OPTION`. Present proposed rulings
to the user and adopt with `quandary adopt NAME --confirmed` only after
confirmation. Never rule in `current` on your own, edit `points/` by
hand, or delete defeated options. Use `quandary reopen` to backtrack.

The settled core, as of the adopted rulings (`quandary rulings` is the
authority; this is a summary):

- Substrate: a new engine with its own pager, not a layer over another
  database or a KV store. The spec states guarantees over constructor
  relations; heaps and pages are implementation.
- Model: a relation is a variable holding a set of values of a nominal
  type. Every relation is a sum; a record is a one-constructor sum.
  Sums are closed and disjoint-total; overlap is relationships to sums.
  No NULL: optionality is a constructor, and Option is a library sum.
  Identity is a declared key, one key across a sum's constructors. A
  reference is the target key typed by its constructor set, only as a
  direct field. No collection fields; many-valued facts are relations.
- Query language: comprehensions over sets with nested results, a
  fixpoint binder, stratified negation, higher-order functions
  normalised before planning. `match` is exhaustive with every
  constructor named (wildcards only on scalars); it means a disjoint
  union of arm queries over constructor relations and executes as one
  union-index probe routed by constructor. No SQL surface.
- Writes: queries are pure; a write is a change set (inserted and
  deleted relations per constructor) applied as one multiple
  assignment. Retag is assigning a value of another constructor.
- Constraints: every constraint is a query that must return nothing,
  holds at every statement boundary, and is admitted only with an
  index-bounded incremental check.
- Indexes, uniques and FK targets range over constructor sets. One heap
  per constructor; union index entries sort as (key, ctor id, tid),
  ctor ids never reused.
- Engine: serializable only, by SSI over in-heap MVCC with 64-bit xids,
  redo-only WAL with full-page images, B-link trees with optimistic lock
  coupling, vmcache over one database file with direct I/O, adaptive
  push-based morsel execution over Arrow batches.
- Evolution: typed schema modification operators closed under inverses,
  scripts as escape hatch; registered queries are rechecked on change;
  readers upgrade before writers use a new constructor. The log holds
  only author-written migrations; bundle retirement and leases are
  deployment state. Bundles (FAMILY-VERSION) pin versions; ad hoc text
  may write, audited.

# adtb

A new database engine with native sum types (algebraic data types) at
both the value level (a column whose type is a union) and the relation
level (an entity set partitioned into constructors, with exclusivity,
totality and foreign keys enforced by the engine).

Rust crate, built with `cargo`. The dev shell is devenv (`devenv.nix`),
entered through direnv.

## Version control

The repo is jj, colocated with git. Use `jj`, not `git`, for commits,
history and branches: `jj status`, `jj diff`, `jj describe -m`,
`jj new`, `jj log`. Commit messages are one subject line in
conventional form (`feat: ...`, `docs: ...`).

## In-progress work

If `work/HANDOFF.md` exists, read it first: it records where the design
work stands and the next steps.

## Tickets

Tracked work lives in `work/issues/` and is managed with `tix`; the
format and priority scheme are in `work/issues/README.md`. Design
decisions are not tickets: they go through the design space. Check
`tix list --ready` before starting work, and run `tix check` after
editing a ticket. `tix` is not on PATH yet
(`work/issues/tix-in-devenv.md`); run it as:

```
nix run path:/home/tony/code/tql/work/tools/tix -- -C work/issues check
```

## Knowledge base

Facts about other databases, formats, languages and papers live in
`work/kb/`, one file per subject, each fact marked `[V date]`, `[S date]`
or `[U]` with its source. Format and rules in `work/kb/README.md`.
Before any WebSearch or WebFetch, grep `work/kb/`; after fetching,
append what was learned. Research agents get the same instruction.

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

## Design space

Design decisions live in a quandary space at `work/research/`:
`questions/`, `notes/` (prior art under `notes/prior/`), `points/`
(rulings). Use the `design-space` skill to work in it. Run `quandary`
from inside `work/research/`; `quandary check` after every edit,
`quandary status` for the frontier, `quandary show ID` for a node.
Rulings go through quandary commands, never by editing `points/`.

Before designing or implementing anything, read the rulings in
`current` (`quandary rulings`). Code follows them; a change that
contradicts a ruling is a design change and goes through the space
first.

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

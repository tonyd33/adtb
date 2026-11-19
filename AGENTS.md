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

## Version control

The repo is jj, colocated with git. Use `jj`, not `git`, for commits,
history and branches: `jj status`, `jj diff`, `jj describe -m`,
`jj new`, `jj log`. Commit messages are one subject line in
conventional form (`feat: ...`, `docs: ...`).

Preserve uncommitted work. Never stash or discard changes to get a clean
tree. Propose a commit subject before committing when a commit is requested.

## Build and checks

Enter the development environment with `direnv allow` or `devenv shell`.
Run commands from the repository root.
For a fresh clone, host prerequisites and optional editor setup, read
[development setup](docs/development.md).

- `cargo build --workspace`: build the workspace.
- `cargo clippy --workspace --all-targets -- -D warnings`: lint Rust.
- `cargo fmt --all --check`: check Rust formatting.
- `cargo xtask test [ARGS]`: run the tests under a memory limit; ARGS go to
  `cargo nextest run`.
- `cargo xtask check [REVSET]`: run every check, including the Loom models,
  Miri, dependency policy, source-size limits, the documentation build with
  its link check, `cargo xtask lint` and the editor grammar's generated sources and tests. With REVSET it
  also checks the lint and structure allowlists against those of the parent of REVSET's root.
- `cargo xtask lint [--against REV] [--counts]`: run the TQL rules under `lint/` with `tql`,
  which the dev shell puts on PATH. `lint/rules.toml` declares each query's file set and rows; a
  zero row prints each site as `FILE:LINE: ROW`, and a ratchet row fails when a file's count
  differs from its entry in `lint/allowlist.toml` (none counts as 0): the counts only fall.
  `--counts` prints the allowlist for the tree; `--against REV` fails on an entry above the
  allowlist at REV, so a rise is a maintainer's act on main. The rows: the comment rules of
  `house.tql`, struct fields visible past their module, decode panics, engine names, sim test
  open helpers and weak refusal assertions (`is_err()`, `is_ok()`, `matches!(.., Err(_))`, a
  substring of error text, a bare `unwrap_err()`, a call of `refused_with`): assert a refusal with
  `adtb_testkit::refused_as`, its value and whole reason.

Use targeted checks while working and run the checks appropriate to the
change before reporting completion.

## Tools

The `xtask` crate's commands and the conformance runner:

- `cargo xtask test [ARGS]`: the full nextest run, output in `target/test.txt`; prints failures,
  the summary and the run's peak memory. `cargo xtask test` and `cargo xtask check` each run in a
  systemd user scope of at most 14G, inside `adtb.slice`, which every run shares, throttled past
  60% of the machine's memory and capped at 75%, no swap. Past a limit the kernel kills a process,
  which nextest reports as SIGKILL, and the run goes on. A typed
  `cargo nextest run` or `cargo test`, the other tools and CI, which has no systemd user manager,
  run without this limit. A test is reported slow at 20 s and killed as hung at 300 s; no test
  may depend on the machine being idle. Never set `RUST_MIN_STACK` where cargo or rustc can see it: it reached rustc and crashed it. Set it only on a spawned test child.
- A test binary caps its own heap: `crates/adtb-heap-cap`, a counting global allocator that the `test-heap-cap` feature links through dev-dependencies, aborts the process past 1.5 GiB of live heap, charging each allocation its malloc chunk. A test that needs more calls `adtb_heap_cap::raise_cap(BYTES)` first and declares `threads-required` in `.config/nextest.toml`, the cap over the default rounded up; `every_raised_test_holds_its_slots` fails otherwise. `ADTB_TEST_HEAP_CAP` overrides everything, in bytes. It holds for a test binary run directly and for the binaries tests spawn. The cap is per process and assumes nextest: plain `cargo test` shares a process across a binary's tests and can abort falsely, so set `RUST_TEST_THREADS=1` for a direct run. `cargo build --workspace` has no cap, but any build with `--all-targets` or `--all-features` does: a `target/debug/adtb` it leaves aborts at the cap and obeys the variable until rebuilt. Miri has none. Page memory, C allocations and freed memory malloc retains are not counted. A new test binary that links neither `adtb-lang` nor `adtb-sys` needs `extern crate adtb_lang as _;`; `every_test_binary_has_the_heap_cap` fails otherwise.
- `cargo conformance [ARGS]`, from the workspace root: the snapshot runner (an alias in
  `.cargo/config.toml` for the `adtb-conformance` binary in the test profile, heap capped, with the `scripts` feature). With no ARGS it runs every conformance file under
  `conformance/` on the reference and the engine and every engine script under
  `crates/adtb-sim/scripts/` on the engine, and prints each failing case with its differences,
  rows as a diff. `--file-name PATH` (a file or a directory, repeatable), `--include PATTERN`,
  `--reference` or `--engine`, `--jobs N`, `--fail-fast`; `--update` writes the rows each
  differing directive got in place of its expected rows, the reference's for a conformance file. The script directives are in the doc
  comment of `crates/adtb-conformance/src/scripted/mod.rs`.
- `cargo conformance probe --schema FILE [--data FILE] [--explain] [--checks NAME] TEXT`: check a text and
  run it on the reference and the engine side by side; `--checks app::c` prints the plan of each
  incremental check of constraint `c`, the text then optional.
- `cargo xtask check` regenerates the grammar in `tools/tree-sitter-adtb/` into
  `target/tree-sitter/generated`, fails on a difference from the checked in `src/` (`scanner.c`
  aside), and runs `tree-sitter test` over its corpus and highlight tests. The tests of
  `adtb-tree-sitter`, in the suite, compare it with `adtb-lang` over the spec and conformance
  texts and the front end fuzzer's programs.
- `cargo xtask check` runs clippy over adtb-sys under `--cfg adtb_loom`, then the Loom models
  of the page state word, its unit tests built with that cfg into `target/loom`, then adtb-sys's
  unit tests under Miri with the nightly toolchain in `ADTB_NIGHTLY_BIN`, which the dev shell
  sets; the stable one stays the default. The two take about 35 s together from cold and 11 s
  warm.
- `cargo xtask check` fails when a non-test source under `crates/` passes 1500 lines.
- `mdbook build`, `mdbook serve` and `lychee --offline --include-fragments` build, serve and
  link-check the documentation site. A new page under `docs/manual`, `docs/spec` or `docs/design`
  goes in `docs/SUMMARY.md`; the suite fails otherwise.

## Invariants and fixes

Before writing code a correctness rule depends on, state the rule and what enforces it: the type system, a lint, a test that searches, or nothing with a written reason. Prefer them in that order.

- Do not stand a string or a position in for identity, or a panic in for an error, or a per-path check in for one gate.
- A bug fix's report states its cause and what stops the next instance of that cause. Fix the cause when it is in reach. If a fix only handles the input in hand, say so.
- A limit states the rule it comes from, not only the value that passes today.

## Test value

A test earns its place by checking behaviour a user or caller relies on, at the level where that contract is stated, that no stronger test already covers. That is a judgement made when writing and reviewing it. Being able to fail is not enough: a test of a constructor fails when an assignment is deleted and is still worthless. Do not break code on purpose to prove a test can fail.

Prefer structure to a test: make a defect impossible or obvious with a type, a single gate or a simpler design, and test only what structure cannot carry. Test observable behaviour through the runners (`cargo conformance` scripts) rather than reaching into internals; a test that needs internals says which observable surface it lacks. A bug fix tests the contract the bug broke across its class, not only the input in hand. Name the contract each new test checks. Every test costs iteration time: it lengthens the Rust build and the suite every change waits on. When a subsystem needs many tests, simplify the subsystem before adding to them.

A test failure is evidence to assess, not an automatic requirement. Before
changing behavior or architecture to satisfy it, identify the governing contract
and whether the defect is in the implementation, expectation or test harness.
Correct tests that preserve bugs or unjustified implementation details. Identify
constraints imposed by the user or the maintainer's adopted rulings separately from track-local
choices; do not silently relax the former. State migration compatibility
constraints separately from lasting contracts; those scope choices can be
reconsidered. Public contract changes still need a ruling from the
maintainer. Record material decisions in the change's description; routine
fixes need no new artifact or approval.

- Assert the exact outcome: the refusal reason and value, not `is_err` or a substring. Do not compare two answers only with each other when both could be the same wrong answer. Do not repeat the code's own predicate as the expected value. An expected value comes from outside the code under test.
- A loop or sweep checks that it ran, and that it reached the case it is for. No `|| true`, ignored results or skipped conditions on what is under test. A script's check must report its failure.
- A test of a mechanism that runs beside the code under test (a worker, a pacer, a sweep, an interleaved command) asserts the mechanism ran where the property needs it. Work it cannot observe, such as work after the last measured step, counts as unmeasured, not as zero.
- A name claims only what the test checks.
- For a general property, write the test that searches: every op, every interleaving, generated pairs, n against 2n. One input from a review confirms that input alone.
- Do not pin a detail no contract states, or test the standard library, a derive or a getter. Do not add what a stronger test in the same binary covers. When removing a test, name its valid property and retained or replacement coverage, or explain why its assertion has no contractual property to preserve.

## Public repository

This repository is public. Keep the maintainer's machine details out of tracked files and commit
messages: disks, partitions, other installs, hardware and home paths. Keep them in untracked
local files.

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
- Before finishing, read the diff and run `cargo xtask lint`.

## Specification

The spec is `docs/spec/`, one file per part; `README.md` there is the
index and assigns each topic to one file. Every part is normative and
states only behaviour users and clients observe, declaratively. Where
sources disagree: the conformance suite, then `semantics.md`, then the
other parts. A spec change that contradicts one of the maintainer's
adopted rulings needs a new ruling from the maintainer first. Examples follow `syntax.md`.

Contracts between subsystems live in `docs/design/`, one file per
boundary; a subsystem relies only on what its counterpart's design
file states. Behaviour inside one subsystem lives with its code. A
rule belongs to the lowest tier that can observe it. Treat any spec
change an implementation asks for as suspect: patch the spec only for
undecided or contradictory observable behaviour, never to promote an
internal choice into a promise.

## Design rulings

Code follows the maintainer's adopted rulings. A change that contradicts
one is a design change and needs a new ruling from the maintainer first.

The settled core, as of the maintainer's adopted rulings (the rulings are
the authority; this is a summary):

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

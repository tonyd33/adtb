# Implementation plan

How the reference implementation gets from the specification in
[spec/](spec/README.md) to a first edition. The crate split, engineering
rules and test methods are in [implementation.md](spec/implementation.md);
this file orders the work. Each milestone ends at a state that can be
tested on its own, and names the spec parts it covers.

## Strategy

Two tracks run side by side and meet at milestone 7.

- Language track: front end, then the reference interpreter and the
  conformance runner. The interpreter is the oracle for every later
  query test, so it comes first, and writing it executes the spec:
  every example that fails to parse or type is a spec bug found before
  the engine depends on it.
- Storage track: simulator, pager, log, heaps, indexes, transactions,
  each built under deterministic simulation from its first line. No
  component runs only on the real disk.

The first release is embedded, matching the workload ruling
(`project/workload/embedded-oltp`): one process, in-process API,
schema evolution and bundles. The server, replication and the derived
and interop features follow.

Spec parts are implemented whole where a milestone names them; where
a milestone takes a subset, the subset is listed. A feature left out
of a milestone is refused with an error naming it, never silently
ignored; builds before the first edition do not claim conformance.

## Decisions for the implementation

These are implementation choices, not spec rulings. Each can change
without touching the spec.

| Topic | Choice |
| --- | --- |
| Toolchain | Rust 1.98, edition 2024, pinned in `rust-toolchain.toml` |
| Parser | Hand-written lexer and recursive-descent parser with error recovery; no parser generator |
| Kernel terms | An arena of typed nodes with interned names, shared by the reference interpreter and the planner |
| Batches | Own Arrow-layout buffers in `adtb` (dense unions, the three special shapes of engine.md §8.2); the `arrow` crate only at the IPC boundary, and only if it passes the allowlist |
| Decimal | Own `i128` scaled arithmetic for `Decimal k` up to 38 digits |
| Text | `unicode-normalization` for NFC, `unicode-segmentation` for graphemes |
| Hashing | `blake3` for version hashes and the ledger; `crc32c` for page and block checksums |
| Encryption | `chacha20poly1305` (XChaCha20-Poly1305), milestone 12 |
| Allowlist | `cargo-deny` with a justification per entry in `deny.toml` |
| Pipelines | Bytecode interpreter only; the machine-code backend stays deferred (engine.md §8.1) |

## Milestones

### 0. Workspace

- Cargo workspace with the six crates of implementation.md §1;
  `src/main.rs` goes, `adtb` becomes a library.
- `rust-toolchain.toml`, `clippy.toml` with the disallowed methods,
  `#![forbid(unsafe_code)]` in every crate except `adtb-sys`,
  `deny.toml`.
- Dev shell gains `cargo-deny` and `cargo-nextest`. Miri and
  `cargo-fuzz` need a nightly toolchain beside the pinned stable one;
  they join the shell at milestones 3 and 1, when their first tests
  exist.
- One check command, `check` in the dev shell (also `devenv test`),
  running build, clippy with warnings denied, `cargo fmt --check`,
  tests, and `cargo deny check`.

Exit: the check command passes on an empty workspace.

### 1. Front end (`adtb-lang`)

Covers syntax.md, typing.md, prelude.md (signatures), model.md (static
rules), semantics.md §2, §3 and §10.

1. Lexer and parser for the whole grammar of syntax.md, including
   schema operators, bundle files, grants and commands. Parsing all of
   it early is cheap and finds grammar gaps.
2. Name resolution: modules, the prelude, the root namespace,
   qualified names, reserved names.
3. Type checker: bidirectional inference, joins, variance, generics
   with bounds, literals, records and constructor sets, `Key r`
   references, totality and partiality.
4. Exhaustiveness and unreachable arms.
5. Desugaring to the kernel calculus, with each desugaring rule of
   semantics.md §10 as a function.

Tests:

- Spec corpus: every fenced example in `docs/spec/` is extracted,
  classified (accepted, refused with a named reason, fragment) and run
  through the front end. Each failure is either a front-end bug or a
  spec bug fixed through the usual spec process.
- Front-end fuzzing (implementation.md §4.5) from day one.
- Typing soundness is checked from milestone 2 on: a well-typed term
  never fails in the interpreter with a type error.

Exit: the spec corpus passes; the fuzzer has run for a fixed budget
with no crash.

### 2. Reference interpreter and conformance runner

Covers semantics.md §4 to §8 for a single schema version, queries.md,
writes.md, constraints.md (naive checking), prelude.md (functions),
interop.md §1 (textual values).

- `adtb-reference`: the naive evaluator of implementation.md §3 over
  `BTreeMap<(RelVarId, CtorId), BTreeSet<Row>>`, with change sets,
  delete actions, upsert, retag, faults, `try`, `fix`, grouping,
  aggregates, ordering and top-n.
- Constraints are checked by evaluating every denial query at each
  statement boundary; admission is not the interpreter's concern.
- Transactions are serial; the interpreter defines the outcome that
  any serializable engine must reproduce.
- Conformance runner over the script format of implementation.md §7,
  and the first suite: one script directory per spec part, starting
  with model, queries, writes and constraints.

Exit: the first suite passes on the interpreter; typing soundness
holds over the spec corpus and over type-directed generated terms.

### 3. Simulation and I/O

Covers implementation.md §2.4, §5.1, §5.5.

- `Io` trait, worker event loop, the three backends (`io_uring` and
  the blocking pool in `adtb-sys`, the simulator in `adtb-sim`).
- Simulated clock, randomness, network and deterministic scheduler.
- `SimStorage` with every fault of engine.md §2.6.
- Loom tests for the page state word.

Exit: a seeded run replays byte for byte; Miri runs the simulator
tests clean.

### 4. Pager and log

Covers engine.md §2 to §4.

- File header on pages 0 and 1, page format, checksums, page kinds.
- vmcache buffer pool: reserved virtual range, frames, eviction,
  optimistic reads through `adtb-sys`.
- Redo-only log with batch framing, segments in `app.adt-log/`,
  full-page images on the first change after a checkpoint, checkpoints,
  recovery.
- Crash replay harness (implementation.md §5.3).

Exit: random page-write workloads under simulated crashes and every
injected fault lose no acknowledged write and report every detectable
corruption; crash replay passes for every flush boundary.

### 5. Heaps and indexes

Covers engine.md §5 and §6.

- Per-constructor slotted heaps, the 24-byte version header, the
  `next` TID and moved flag, heap ids.
- Order-preserving key encoding for every key type, `ovarint`
  constructor ids, union index entries `(key, ctor id, tid)`.
- B-link trees with optimistic lock coupling; Loom tests for the
  latch protocol.
- Vacuum: batched dead sets, heap pruning, index cleanup.
- `verify` structural checks.

Exit: model-based tests against `BTreeMap` under simulation; `verify`
clean after every crash.

### 6. Transactions

Covers transactions.md, engine.md §7, semantics.md §8.

- 64-bit xids, commit log, snapshots, MVCC visibility.
- SSI: SIREAD locks on heaps and next-key ranges, rw-antidependency
  tracking, safe snapshots, serialization failures at snapshot edges.
- Commit path with group commit; durability level local.
- Isolation checker (implementation.md §5.4) on the append workload.

Exit: no G0, G1 or G2 cycle over a fixed number of seeds per run;
the write-skew tests abort one side.

### 7. Engine, single version

The tracks meet. Covers model.md, queries.md, writes.md,
constraints.md and transactions.md in the engine, catalog.md §1, §2
and §4 (subset needed to bootstrap), engine.md §8, and embedding.md
§1, §2 (no migrations), §5 and §6.

- Bootstrap catalog and system relations with their fixed ids.
- Schema from declarations applied as one first migration (no other
  operators yet).
- Planner from kernel terms: constructor-relation scans, union index
  probes routed by constructor, one pipeline per `match` arm, joins,
  grouping, aggregates, semi-naive `fix`, top-n, and the rewrites that
  the plan-toggle tests switch off.
- Bytecode pipeline executor over batches, morsel scheduler.
- Write path: change sets, retag across heaps, delete actions, upsert,
  key generation (`sequence`, `uuidv7`), defaults.
- Constraints with the admission rule and incremental checks over
  base indexes.
- In-process API: `Database::open`, ad hoc text, `transact`,
  interactive transactions, retry.

Exit: the whole single-version suite passes on the engine; the
differential, constructor-partition, predicate-partition and
plan-toggle tests (implementation.md §4.1 to §4.4) pass over a fixed
number of seeds per run.

### 8. Evolution and bundles: embedded alpha

Covers evolution.md, applications.md, embedding.md in full except §12,
semantics.md §9, security.md §1 and §4 (principals and definer rights
in process), interop.md §9 (host functions).

- Operator log, version hashes, every schema operator with its data
  map, inverse and declared loss; revert.
- Write states, readers-first, pending drops, staged jobs.
- Bundles and families, registration checks, pinning, translation of
  pinned queries, leases, supersession at open, `hide new
  constructors`.
- Registered queries and transactions with definer rights.
- Embedded open steps, host functions, bulk append.
- Rust code generation from a bundle file (typed calls).
- Schema operator law tests and the schema walk generator
  (implementation.md §4.6, §4.7).

Exit: an application can ship, evolve and reopen an embedded database
through the in-process API alone; operator laws hold over generated
databases. This is the first release.

### 9. Operations: embedded beta

Covers catalog.md §3, §5 to §13, interop.md §6 and §7 (dump, restore,
backup), engine.md §10.

- Statistics, plan pinning with last-good fallback, `explain`.
- `verify` cross and logical checks, scrub, vacuum scheduling,
  resource control and budgets, configuration, metrics.
- Zone rules and collations in the catalog.
- Dump, restore, online backup, point-in-time restore.
- Fault-injecting file system tests on the real backends
  (implementation.md §5.2).
- Benchmarks against SQLite on the embedded OLTP workloads.

Exit: the suite and every test method pass; benchmarks are published
with the release.

### 10. Server

Covers wire.md, security.md §2, §3, §5 to §8, catalog.md (the server
parts).

- `adtb-server`: transports, framing, handshake and binding, calls,
  transactions, ad hoc text, appends, admission, feeds, pushes,
  idempotency, compression, protocol versions.
- Roles, authentication sources, grants, peer mappings, audit with
  the ledger.
- Real-process isolation checks under kills and pauses.

Exit: the parity tests between the in-process API and the wire pass
(wire.md §14, embedding.md §11).

### 11. Replication

Covers engine.md §9, evolution.md §9, embedding.md §12,
transactions.md §8.

- Log shipping, replicas, safe snapshot markers, replica reads,
  vacuum feedback, lease handling on replicas, forwarding from
  embedded replicas, quorum durability.

Exit: simulated partitions and failovers lose no acknowledged commit
at quorum durability, and replica reads stay serializable.

### 12. Derived features and interop

Covers derived.md, interop.md (the rest), security.md §9 to §11.

- Views, stored views with in-transaction maintenance, computed
  fields, time travel with history heaps and `at commit`.
- External formats, file scans, loading with rejects, export, change
  streams.
- Wasm user-defined functions and aggregates, derived index kinds
  (full-text search).
- SQL schema migration tool.
- Encryption at rest, tenancy and the fleet tool.

### 13. First edition

- The conformance suite covers every normative part.
- The reference interpreter covers evolution and isolation outcomes
  needed by the suite.
- Spec, interpreter and suite are released together as the first
  dated edition.

## Order of work

Milestone 0 first. Then milestones 1 and 3 start together and each
track proceeds in order (1, 2 and 3, 4, 5, 6). Milestone 7 needs 2 and
6. Milestones 8 to 13 are sequential, except that 12's derived
features may start once 8 is done.

## Tracking

Each milestone becomes a list of tasks when it starts, not before.
Spec bugs found while implementing go through the spec process
(design space first when they contradict a ruling) and are fixed in
the spec, not worked around in code.

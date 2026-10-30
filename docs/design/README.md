# adtb design

This directory states the contracts between the subsystems of the
reference implementation. The specification in [../spec/](../spec/README.md)
states what users and clients observe; this directory states what one
subsystem provides to another and what the other may rely on.

## 1. Tiers

The project's documentation has three tiers.

The first tier is [../spec/](../spec/README.md). It is normative and
states the behaviour users and clients observe: what a query means,
what a commit guarantees, what an error carries, what the wire and the
in-process API accept.

The second tier is this directory. It states the contracts between
subsystems, meaning crates and major components, which only other
subsystems observe. A subsystem may rely on what its counterpart's
design file states, and on nothing else of it. A user cannot observe a
second-tier fact except through its first-tier consequences.

The third tier is what stays inside one subsystem. It lives with the
code. Until the code exists, third-tier notes sit at the end of the
relevant design file under the heading "Notes for the implementation".
They are not binding: the subsystem may change them without telling
anyone.

"Must" in a design file marks an obligation of the side that provides
the contract. A precondition is what the caller must establish; a
postcondition or an invariant is what the caller may rely on.

## 2. Shape of the engine

The engine is a database engine of its own, not a layer over another
database or a key-value store. It owns its pager, its heap and index
formats, its log and its buffer pool. Its core is a library linked into
an application; the server of [../spec/wire.md](../spec/wire.md) is a
client of that library.

The engine targets an application database inside one process, with
many short transactions, and data many times larger than memory on one
node. Linux is the tuned platform; the I/O layer has portable backends
so the library embeds on other systems with slower paths
([runtime.md §2](runtime.md#2-io-interface)).

Rulings: substrate/new-engine, encoding/native, engine/guarantees-only, engine/own-pager, engine/embedded-server, project/workload/embedded-oltp, project/scale/out-of-core, buffer/platform/linux-first

## 3. Crates and boundaries

The implementation is split into crates only where a part has
different dependencies, consumers or stability promises:

| Crate | Contents | Depends on |
| --- | --- | --- |
| `adtb-lang` | Lexer, parser, type checker, exhaustiveness, desugaring to the kernel, the textual value syntax ([kernel.md §9](kernel.md#9-textual-syntax)); no I/O | standard library, Unicode normalisation and regular expression syntax |
| `adtb-file` | The readers of CSV and NDJSON files at a declared row type, over any value type of the textual syntax, and the files a path names ([oracle.md §1](oracle.md#1-evaluation-contract)) | `adtb-lang`, Unicode normalisation |
| `adtb-reference` | The reference interpreter ([oracle.md](oracle.md)) | `adtb-lang`, `adtb-file`, Unicode, collation, regular expression and zone tables |
| `adtb-sys` | Every `unsafe` block: page memory, direct I/O buffers, `io_uring`, atomic page reads, the CRC-32C instruction ([sys.md](sys.md)); the runtime interfaces: I/O, clock, threads, entropy ([runtime.md](runtime.md)) | `libc` and allowlisted system crates |
| `adtb` | Engine core: catalog, planner, executor, transactions, storage, over the I/O interface of [runtime.md](runtime.md) | `adtb-lang`, `adtb-file`, `adtb-sys`, and `adtb-reference` and `adtb-testkit` under the `plan-fixture` feature, which only the tests of `adtb-sim` enable |
| `adtb-sim` | Simulated storage, network, clock, randomness and scheduler, fault injection ([runtime.md §5](runtime.md#5-simulated-storage)), and the tests that run the engine over them; only tests and the `adtb-probe` tool depend on it | `adtb` |
| `adtb-simkit` | The helpers the simulator's test binaries share; only tests depend on it | `adtb`, `adtb-sim`, `adtb-lang`, `adtb-reference`, `adtb-testkit`, `adtb-isolation` |
| `adtb-conformance` | The conformance runner over the script format of [../spec/conformance.md](../spec/conformance.md); the suite itself is `conformance/` at the root, one directory per spec part | `adtb-lang`, `adtb-reference`, and `adtb` once the engine runs scripts |
| `adtb-isolation` | The isolation checker over list-append histories and the scripted anomaly scenarios ([runtime.md §6](runtime.md#6-isolation-histories)) | standard library |
| `adtb-server` | The wire protocol of [../spec/wire.md](../spec/wire.md) around `adtb` ([server.md](server.md)) | `adtb` |
| `adtb-codegen` | Rust code generation from a migration log and bundle files ([kernel.md §8](kernel.md#8-code-generators)) | `adtb-lang` |
| `adtb-codegen-tests` | The code generated from fixture schemas and bundles, compiled and called over the simulator | `adtb`, `adtb-codegen` at build time |

Layers inside `adtb` (catalog, planner, transactions, storage) are
modules, not crates. Their boundaries are still contracts and have
design files of their own.

| Boundary | Provider | Consumers | Design file |
| --- | --- | --- | --- |
| Typed kernel terms | `adtb-lang` | `adtb`, `adtb-reference`, code generators | [kernel.md](kernel.md) |
| Textual value syntax | `adtb-lang` | `adtb`, `adtb-reference` | [kernel.md §9](kernel.md#9-textual-syntax) |
| File records | `adtb-file` | `adtb`, `adtb-reference` | [oracle.md §1](oracle.md#1-evaluation-contract) |
| Safe page memory and I/O buffers | `adtb-sys` | `adtb` | [sys.md](sys.md) |
| I/O, time, threads, randomness | I/O backends, `adtb-sim` | `adtb` | [runtime.md](runtime.md) |
| File, pages, log, recovery | pager and log | heaps, indexes, replication, backup, verify | [storage.md](storage.md) |
| Structural verification | `verify` | catalog, maintenance commands, test harnesses | [verify.md](verify.md) |
| Row versions and isolation | transactions | heaps, indexes, executor, vacuum | [mvcc.md](mvcc.md) |
| Index entries and keys | indexes | heaps, transactions, planner | [indexes.md](indexes.md) |
| Physical plans and batches | planner | executor, catalog | [execution.md](execution.md) |
| Constraint and view maintenance | constraint and view compiler | transactions | [maintenance.md](maintenance.md) |
| Staged schema changes | evolution | catalog, planner | [schema-change.md](schema-change.md) |
| Log stream and replicas | primary | replicas, embedded replicas, backups | [replication.md](replication.md) |
| Calls over the wire | `adtb-server` | `adtb` | [server.md](server.md) |
| Typed bundles | `adtb` | generated code | [server.md §4.5](server.md#45-typed-bundles) |
| Histories and anomalies | test harnesses | `adtb-isolation` | [runtime.md §6](runtime.md#6-isolation-histories) |
| The test oracle | `adtb-reference` | test harness, `adtb-conformance` | [oracle.md](oracle.md) |

Rulings: project/arch/crates/by-boundary

## 4. Errors

Errors split in two:

- Operating errors are values of `Result`: the disk refused a read,
  a page failed its checksum, a query exceeded its memory budget, a
  transaction hit a serialization failure. Each is handled, and those
  that reach a client are values of the error sums in
  [../spec/catalog.md §4](../spec/catalog.md#4-error-sums).
- A broken invariant is a bug in the engine. What it does depends on
  what the invariant is over:
  - An invariant over engine state (pages, the log, latches, heap and
    index layout, the stored catalog): the process crashes and the
    database recovers from the log.
  - A guarantee the checker makes about a submitted program (the types
    and normal forms that reach planning, evaluation and write
    evaluation): the statement fails with an internal error, its
    effects are discarded, the transaction stays open and the process
    keeps running. A client receives it as the `Internal` constructor
    of the statement error sum.

A failed write or `fdatasync` of the log or a page is in the second
class. It is not retried. The engine crashes and recovers from the log:
the failed pages are rebuilt from their last durable image plus replay
([storage.md §7](storage.md#7-recovery)).

```text
fn read_page(&mut self, pid: Pid) -> Result<PageRef, IoError>   // the disk said no
assert!(page.lsn() <= self.flushed_lsn);                        // the engine is wrong
```

A subsystem may rely on its counterpart never returning a value that
breaks the counterpart's stated invariants: such a state crashes the
process, or for a checker guarantee fails the statement, before it is
returned.

Rulings: project/arch/errors/fatal-split,
project/arch/errors/checker-guarantee/fail-statement,
buffer/fsync-fail/panic-replay

## 5. Glossary

| Term | Meaning |
| --- | --- |
| constructor id | The number of a declared constructor of a type, as [../spec/wire.md §4.3](../spec/wire.md#43-records-and-sums) assigns it. |
| heap | The pages holding every row version of one constructor relation, keyed by (relation variable, constructor id). Two relation variables of one type have separate heaps. |
| heap id | The number of one heap, current or history ([mvcc.md §10](mvcc.md#10-reclamation)), assigned when the heap is created and never reused. |
| TID | The address of a row version within its heap: a page id and a slot ([mvcc.md §2](mvcc.md#2-tids)). |
| constructor-set index | A B-link tree or hash index over the rows of a set of constructors of one relation variable ([indexes.md](indexes.md)). |
| entry | An index element `(key, ctor, tid)`. |
| key index | The constructor-set index over every constructor of a sum that covers the sum's key. |
| xid | A 64-bit transaction id, never reused. |
| LSN | A byte position in the redo log. |
| commit position | A `Position { epoch, lsn }`: the primary epoch and the LSN of a transaction's commit record. |
| sum | A relation variable with all its constructor heaps and indexes. |

## 6. Running example

Examples use the running schema of
[../spec/README.md](../spec/README.md#conventions) at version 12, after
`Apple` is added and enabled. The engine sees it as one sum `Identity`
with three constructor heaps, `identities.EmailPass`,
`identities.Google` and `identities.Apple`, and three constructor-set
indexes:

```adt
relation identities : Identity
  key id generate uuidv7       -- index over {EmailPass, Google, Apple}
  unique (email) in EmailPass  -- index over {EmailPass}
  unique (gid) in Google       -- index over {Google}
  index (user)                 -- local extension: index over {EmailPass, Google, Apple}
```

## 7. Files

- [kernel.md](kernel.md): the typed kernel terms `adtb-lang` hands to the engine, the reference interpreter and code generators, and the textual value syntax over either's values.
- [sys.md](sys.md): the safe types `adtb-sys` exposes and the unsafe invariants behind them.
- [runtime.md](runtime.md): the I/O interface, the worker loop, time, threads, randomness, and the faults simulated storage injects.
- [storage.md](storage.md): the file, pages, log records and blocks, write-ahead rules, checkpoints, recovery, encryption sealing and format versions.
- [mvcc.md](mvcc.md): heaps per constructor, version headers, the commit log, chains, retag, uniqueness checks, SIREAD locks, the watermark and reclamation.
- [indexes.md](indexes.md): index entries, key encoding, order keys, hashing, indexes on sum fields, hash indexes and index builds.
- [execution.md](execution.md): physical plans, batch layouts for sums, routing and fusion, pruning, stored plans and replanning.
- [maintenance.md](maintenance.md): delta queries for constraints and stored views, derivation counts and commutative increments.
- [schema-change.md](schema-change.md): staged schema jobs, constructor and index states, rechecks and host function binding.
- [replication.md](replication.md): the log stream, safe snapshot markers, vacuum feedback, epochs, logical records and backups.
- [server.md](server.md): how `adtb-server` drives the engine.
- [oracle.md](oracle.md): the reference interpreter's contract with the test harness and the conformance runner.
- [verify.md](verify.md): the structural `verify` of a whole file, the inventory its caller supplies, the repair of leaked pages, the `verify` command under writers, and the rows of volatile relations kept in the file.

## 8. Notes for the implementation

Non-binding.

Cheap assertions run in every build, release included. Expensive
checks, such as verifying a whole B-link tree's order or a heap
against its indexes, run in debug and simulation builds only.

```text
assert!(slot < page.slot_count());
check_expensive!(tree.verify_order());
```

The Rust toolchain is pinned in `rust-toolchain.toml` and by the dev
shell; the pinned version is the minimum supported Rust version.
Raising it is a deliberate commit.

Rulings: project/arch/assertions/tiered, project/arch/msrv/pinned

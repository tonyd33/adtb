# Implementation

This annex describes how the reference implementation is built and
tested: its crates, the rules its code follows, the reference
interpreter used as a test oracle, the testing methods for query
results, storage and isolation, the concurrency it is built for, its
non-goals, and how the conformance suite is run. It is informative.
"Must" here marks a rule the implementation's own tooling enforces.
The engine it describes is in [engine.md](engine.md).

## 1. Crates

The implementation is split into crates only where a part has
different dependencies, consumers or stability promises:

| Crate | Contents | Depends on |
| --- | --- | --- |
| `adtdb-lang` | Lexer, parser, type checker, exhaustiveness, desugaring to the kernel; no I/O | standard library |
| `adtdb-reference` | The reference interpreter (§3) | `adtdb-lang` |
| `adtdb-sys` | Every `unsafe` block: page memory, direct I/O buffers, `io_uring`, atomic page reads (§2.2) | `libc` and allowlisted system crates |
| `adtdb` | Engine core: catalog, planner, executor, transactions, storage, over the I/O interface of §2.4 | `adtdb-lang`, `adtdb-sys` |
| `adtdb-sim` | Simulated storage, network, clock, randomness and scheduler, fault injection (§5.1), and the tests that run the engine over them; nothing depends on it | `adtdb` |
| `adtdb-server` | The wire protocol of [wire.md](wire.md) around `adtdb` | `adtdb` |

Code generation tools for host languages depend on `adtdb-lang` alone.
They compute type, constructor and relation variable ids from the
operator log ([wire.md](wire.md#43-records-and-sums)) and call ordinals
from the order of `query`, `txn`, `import fn` and `feed` declarations
in the bundle file ([wire.md](wire.md#51-call)), so generated code
agrees with every database built from the same log.
Layers inside `adtdb` (catalog, planner, transactions, storage) are
modules, not crates.

Rulings: project/arch/crates/by-boundary

## 2. Engineering rules

### 2.1 Dependencies

Dependencies come from a short audited allowlist, each entry with a
written justification. Only dependencies that take no ambient time,
threads or randomness are admitted: every thread, timer and random draw
goes through the simulator's interfaces (§5.1), or a seed would not
replay. Pure libraries (checksums, hashing, decimal arithmetic) pass;
runtimes do not. The rule is checked by lints:

```
# clippy.toml
disallowed-methods = ["std::time::Instant::now", "std::time::SystemTime::now",
                      "std::thread::spawn", "rand::thread_rng"]
```

The engine's interfaces for time, threads and randomness are the only
callers of the real implementations. Dev-dependencies (property
testing, Loom, fuzz targets) are allowlisted separately.

### 2.2 Unsafe code

`unsafe` appears only in `adtdb-sys`, which states its contract in its
documentation and exposes safe types (`Frame`, `PageRef`,
`AlignedBuf`) to the rest of the engine. Every other crate has
`#![forbid(unsafe_code)]`.

The contract covers:

- Page memory is reserved once and never unmapped while the database
  is open, so a page address never dangles; eviction only releases the
  physical memory behind it.
- A buffer handed to the kernel for I/O stays pinned until its
  completion is processed, whatever happens to the work that requested
  it.
- Optimistic reads of a page another thread may write go through atomic
  loads into local copies; no reference to such a page is formed.
- Every length or offset read optimistically is bounds-checked before
  use.

Miri runs the whole engine over the simulated storage of §5.1, which is
plain Rust, through the tests of `adtdb-sim`, so the pager, latches and page layouts are checked for
undefined behaviour and data races. The real I/O backends, which Miri
cannot execute, are tested natively under AddressSanitizer and
ThreadSanitizer.

```
$ cargo miri test -p adtdb-sim
```

### 2.3 Memory

The page pool is a fixed size chosen at open. Query memory comes from
arenas allocated on demand within a per-query budget and freed whole
when the query ends; an arena request past the budget fails with an
operating error.

```
let arena = budget.arena(query_id, limit)?;
let table: HashTable<'_, K, V> = HashTable::new_in(&arena);
```

### 2.4 Runtime and I/O loop

The engine has no async runtime dependency. Each worker thread runs
the engine's own event loop over one I/O interface, with three
implementations: `io_uring`, a blocking thread pool, and the simulator.

```
trait Io {
  fn submit(&mut self, op: Op) -> Ticket;
  fn poll(&mut self) -> Option<Completion>;
}
struct Worker<I: Io> { io: I, morsels: Queue, ... }
```

The simulator controls every thread and every completion through this
interface.

### 2.5 Errors

Errors split in two:

- Operating errors are values of `Result`: the disk refused a read,
  a page failed its checksum, a query exceeded its memory budget, a
  transaction hit a serialization failure. Each is handled, and those
  that reach a client are values of the error sums in
  [catalog.md](catalog.md#4-error-sums).
- A broken invariant is a bug in the engine. The process crashes and
  the database recovers from the log, as it does after a failed
  `fdatasync` ([engine.md §3.6](engine.md#36-write-and-flush-failures)).

```
fn read_page(&mut self, pid: Pid) -> Result<PageRef, IoError>   // the disk said no
assert!(page.lsn() <= self.flushed_lsn);                        // the engine is wrong
```

### 2.6 Assertions

Cheap assertions run in every build, release included. Expensive
checks, such as verifying a whole B-link tree's order or a heap
against its indexes, run in debug and simulation builds only.

```
assert!(slot < page.slot_count());
check_expensive!(tree.verify_order());
```

### 2.7 Toolchain

The Rust toolchain is pinned in `rust-toolchain.toml` and by the dev
shell; the pinned version is the minimum supported Rust version.
Raising it is a deliberate commit.

Rulings: project/arch/deps/allowlist, project/arch/deps/escape/no-ambient, project/arch/unsafe/confined, project/arch/unsafe/miri-io/sim-storage, project/arch/unsafe/miri-io/sys-tests, project/arch/alloc/budgeted, project/arch/runtime/own-loop, project/arch/errors/fatal-split, project/arch/assertions/tiered, project/arch/msrv/pinned

## 3. Reference interpreter

The reference interpreter is a naive evaluator of the kernel calculus
of [semantics.md](semantics.md) over in-memory sets. It shares nothing
with the engine except the typed kernel terms of `adtdb-lang`: no
indexes, no plans, no pages. A database is a map from (relation
variable, constructor id) to a set of rows, so two relation variables
of one type stay apart. A sum is the union of its constructor
relations, a `match` the union of its arms, a change set the inserted
and deleted relations per constructor.

```
fn eval(q: &Kernel, db: &BTreeMap<(RelVarId, CtorId), BTreeSet<Row>>) -> BTreeSet<Value>
```

It answers what any generated query over any generated database should
return. It is released with each edition of the spec, beside the
conformance suite (§7).

The interpreter and the engine both consume generated typed kernel
terms, so differential tests (§4.1) check planning and execution, not
the front end. The front end is tested on its own:

- Desugaring: a surface term and its desugaring give equal results in
  the interpreter.
- Typing: a well-typed term never fails at run time with a type error.
- Fuzzing (§4.5): malformed input is refused with a typed error, never
  a crash.

Float results are compared within a bound derived from the input: two
results of an aggregate over `n` values `x_i` agree when they differ by
at most `n * eps * sum(|x_i|)`, with `eps` the machine epsilon of the
float type. Every other value is compared exactly.

Rulings: project/oracle/rust-reference, project/oracle/shared-front/typed-input, project/oracle/float/tolerance

## 4. Testing query results

### 4.1 Differential testing

A seed generates a schema, a database and a query; the engine's result
must equal the reference interpreter's, as sets, with nested results
compared as nested sets.

```
seed -> (schema S, database D, query q)
assert engine(q, D) == reference(q, D)
```

### 4.2 Constructor partitioning

A sum is the disjoint union of its constructor relations, so a query
over the sum equals the union of the same query over each constructor:

```
from i in identities where p(i) select f(i)
  ==  (from i in identities.EmailPass where p(i) select f(i))
    union (from i in identities.Google where p(i) select f(i))
    union (from i in identities.Apple where p(i) select f(i))
```

The left side runs through the key index and merged heap scans; the
right side through single-heap scans and constructor pruning. The same
holds for each `match` and the union of its arms. This checks the
planner without the reference interpreter.

### 4.3 Predicate partitioning

A query equals the union of its restrictions to `p` and to `not p`.
There is no NULL and `Bool` has two constructors, so two partitions
cover every row. Aggregates decompose the same way:

```
count(x for x in g) == count(x for x in g where p) + count(x for x in g where not p)
```

### 4.4 Plan toggles

Every query gives the same result with any subset of the planner's
rewrites disabled: arm distribution of joins, constructor pruning,
probe fusion, and compiled versus interpreted pipelines.

```
for flags in subsets(rewrites): assert run(q, D, flags) == run(q, D, none)
```

### 4.5 Front-end fuzzing

Coverage-guided fuzzing drives the parser, the type checker and the
wire decoder with arbitrary bytes. Every input is either accepted or
refused with a typed error; none crashes.

### 4.6 Schema operator laws

For every schema operator with an inverse, `inv(op)(op(D)) == D`; for
an operator with a declared loss, the law holds on the declared
projection. For a query `q` pinned to version `v` and its translation
`q'` at the version `op` produces, `q(D) == q'(op(D))`.

The example starts from a version of a local schema in which `Login`
has a constructor `Social`:

```
type Provider = Google | Apple
type Login { id: Key logins } = Password { hash: Bytes }
                              | Social   { provider: Provider, sub: Text }
relation logins : Login key id generate sequence

-- 0002
split constructor Login.Social into Google { gid: Text } | Apple { sub: Text }
  by match
    | Social { provider: Google, sub } -> Google { gid: sub }
    | Social { provider: Apple, sub }  -> Apple { sub: sub }
-- law: the inverse merge, keeping the tag as provider, restores D
```

### 4.7 Generating inputs

Inputs are generated well-typed, in three ways:

| Method | What it generates |
| --- | --- |
| Type-directed | A sum, its constructors and fields, rows per constructor heap, then a query built rule by rule from the typing rules, choosing `match` arms from the scrutinee's constructor set; failures shrink to a small schema and query |
| Corpus mutation | Subterms of the conformance suite and registered bundles mutated, keeping the mutants that still type-check |
| Schema walk | Random sequences of schema operators applied to a populated database, interleaved with writes and queries, re-registering bundles after each; the reference interpreter checks every query before and after |

A schema walk exercises recompilation of registered queries, lazy page
format rewrites and staged schema jobs.

Rulings: project/testing/differential, project/testing/ctor-partition, project/testing/tlp, project/testing/plan-toggle, project/testing/front-fuzz, project/testing/smo-laws, project/testing/gen/type-directed, project/testing/gen/corpus-mutation, project/testing/gen/schema-walk

## 5. Testing storage, concurrency and isolation

### 5.1 Deterministic simulation

The whole engine runs inside a simulator that supplies time, I/O,
network, randomness and thread scheduling through the interfaces of
§2.4. A deterministic scheduler replaces the operating system's
scheduling of the engine's threads, so morsel workers, the pager and
SSI run multi-threaded under it; a seed replays a run exactly.

The simulated storage is an in-memory disk that injects every fault of
the engine's fault model ([engine.md §2.6](engine.md#26-corruption-detection)):

```
SimStorage {
  seed,
  fault: { torn_p, corrupt_p, misdirect_p, lost_write_p,
           lose_unsynced: true, delay_completions: true },
}
```

After each run, and after each simulated crash and recovery, the
simulator checks that every acknowledged commit is present, that no
unacknowledged state is reported as committed, and that `verify`
finds no structural, cross or logical violation
([catalog.md](catalog.md#111-verification)).

### 5.2 Fault injection under the real engine

A fault-injecting file system runs beneath the real binary and
reproduces how ext4, XFS and Btrfs behave after a failed `fsync`,
including pages marked clean after the failure. It tests the real I/O
backends, which the simulator replaces.

### 5.3 Crash replay

The block-level stream of writes and flushes between the engine and
the device is recorded. For each flush boundary, and each subset of
the writes issued since the last flush, a disk image is built and
recovery runs on it; the checks of §5.1 must hold for every image.

### 5.4 Isolation checking

Histories of concurrent transactions are checked for Adya's phenomena.
The promise is serializability, so any G0, G1 or G2 cycle in a history
is a bug. The workload appends one row per element, so every read
shows which appends are visible:

```
type Elem = { id: Key elems, k: I64, e: I64 }
relation elems : Elem
  key id generate sequence
  unique (k, e)

txn append (k: I64, e: I64) -> Key elems =
  transact
    let x = insert elems { k: k, e: e }
    return key x
query read_elems (k: I64) -> Set I64 = from x in elems where x.k == k select x.e
```

Histories come from two places:

- Simulated clients inside the simulator run registered transactions;
  a cycle comes with the seed that produced it.
- Real server processes are killed, paused and restarted while clients
  drive them over the wire protocol, which finds what the simulation
  does not model.

### 5.5 Memory model

The simulated scheduler interleaves threads at yield points but runs on
the host's memory ordering, so weak-memory reorderings are tested
separately. Loom explores every interleaving of small latch tests (the
page state word, optimistic lock coupling) under the C11 model; Miri
runs the simulated engine and reports data races and undefined
behaviour (§2.2).

```
#[cfg(loom)] #[test]
fn olc_reader_sees_consistent_or_restarts() {
  loom::model(|| { /* a writer thread and a reader thread on one page */ });
}
```

Rulings: buffer/test/dst, buffer/test/dst-threads/sim-scheduler, buffer/test/fault-fs, buffer/test/crash-replay, project/testing/isolation-check, project/testing/elle/row-per-element, project/testing/where/in-sim, project/testing/where/real-process, project/testing/memory-model/loom, project/testing/memory-model/miri

## 6. Concurrency model and non-goals

The engine is built for tens of threads of one application calling the
in-process API ([embedding.md](embedding.md)). A connection is a
handle, not a server resource. One operating system process owns a
database file; another process that opens it is refused, and reaches
the database through the server instead.

```rust
let mut tx = app.begin(&TxnOptions::default())?;
tx.call(queries::sign_up, (n, g), &TxnCallOptions::default())?;
tx.commit()?;   // may fail with a serialization failure; the caller retries
```

`app` is a `Bundle` bound by `Database::bundle`
([embedding.md](embedding.md)).

The project does not build:

| Non-goal | Consequence |
| --- | --- |
| Distributed databases | No sharding and no distributed transactions; one database is one node with optional replicas |
| SQL compatibility | No SQL dialect, no SQL front end, no Postgres or SQLite wire protocol |
| Several writing processes | No sharing of one file between processes through file locks |
| A columnar storage format | Per-constructor row heaps are the only format; analytic speed comes from execution |

Rulings: project/concurrency/app-threads, project/nongoals/distributed, project/nongoals/sql-compat, project/nongoals/multi-process, project/nongoals/columnar-store

## 7. Conformance suite

Each edition is released as prose, the reference interpreter and the
conformance suite together. The suite is normative. Where they
disagree, a suite test wins over [semantics.md](semantics.md), and
semantics.md wins over the surface parts, until the next edition
corrects one of them ([README.md](README.md#conformance)).

A test is a script of statements and queries with expected rows:

```
schema
  module test
  type Login { id: Key logins } = Password { email: Text } | Social { sub: Text }
  relation logins : Login key id generate sequence
run
  transact
    insert logins.Social { sub: "s1" }
query rowsort
  from l in logins select match l | Password p -> p.email | Social s -> s.sub
----
"s1"
error Invalid
  from l in logins select match l | Social s -> s.sub
```

| Directive | Meaning |
| --- | --- |
| `schema` | Declarations applied to an empty database as one migration |
| `run` | Statements executed in order; each must succeed |
| `query` | A query whose result rows follow `----`, one per line in textual value syntax; `rowsort` compares them as a set |
| `error C` | A statement or query that must be refused with the reason constructor `C` of [catalog.md](catalog.md) (here `Invalid`: the text fails to type-check) |

A runner executes each script against a fresh database, once against
the engine and once against the reference interpreter, and reports
every directive whose outcome differs from the script. Values are
written in the textual syntax of [interop.md](interop.md#1-textual-value-syntax).

Rulings: project/spec/conformance/normative-suite, project/spec/test-format/script, project/spec/shape/executable

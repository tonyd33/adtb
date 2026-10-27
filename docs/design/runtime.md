# Runtime

The engine in `adtb` reaches the operating system only through the
interfaces of this file. Each has a real implementation and a simulated
one in `adtb-sim`; the engine may rely on nothing of either beyond what
is stated here.

## 1. Time, threads and randomness

The engine has interfaces for time, threads and randomness, and they
are the only callers of the real implementations. Every thread, timer
and random draw of the engine goes through them, so a simulated run is
replayed exactly by its seed (§5).

Precondition on every dependency of `adtb`: it takes no ambient time,
threads or randomness. Pure libraries (checksums, hashing, decimal
arithmetic) meet it; runtimes do not.

The interfaces, with the I/O interface of §2, are defined in
`adtb-sys` as the module `runtime`, so that the backends `adtb-sys`
holds implement them, and `adtb` re-exports the module as
`adtb::runtime`. The engine is generic over one `Runtime`, a cloneable
handle that every thread of the engine shares:

```text
trait Clock   { fn now(&self) -> Instant; fn wall(&self) -> SystemTime; }
trait Threads { fn spawn(&self, name, body) -> ThreadId; fn current(&self) -> ThreadId;
                fn park(&self); fn unpark(&self, ThreadId); fn yield_now(&self);
                fn join(&self, ThreadId); }
trait Entropy { fn next_u64(&self) -> u64; fn fill(&self, &mut [u8]); }
trait Runtime: Clock + Threads + Entropy + Clone + Send + Sync { type Io: Io; fn io(&self) -> Self::Io; }
```

Postconditions every runtime must meet:

- `now` never goes backwards within one process. An `Instant` is a
  count of nanoseconds from an origin the runtime picks, and means
  nothing in another process; it is never written to the file.
- `wall` may jump in either direction.
- `park` blocks the calling thread until its wake token is set, then
  clears the token. `unpark` sets a thread's token. A token set before
  the `park` is not lost; `park` may also return spuriously.
  `unpark` of a thread that has returned does nothing.
- `join` returns once the thread has returned from its body.
- A panic in any thread crashes the process
  ([README.md §4](README.md#4-errors)).

Preconditions on the engine:

- A thread that waits on memory another thread writes, a spin on a
  latch among them, calls `yield_now` or `park` on every iteration.
  The simulator switches threads only inside calls to these
  interfaces and, when its configuration enables them, at the switch
  points of page memory ([sys.md §1](sys.md#1-safe-types)), so a wait
  that never calls them never ends there.
- Every call into these interfaces, and every page memory switch
  point, may hand the processor to another thread or unwind the
  calling thread on a crash. The engine leaves no in-memory structure
  half-updated across one: another thread may observe it there, and a
  crash may stop the thread there with only its destructors run.
- No destructor blocks on these interfaces. A crash stops every thread
  wherever it is; the simulator unwinds them and runs destructors, and
  a destructor that waits for a completion cannot be answered.

Timers are ops of the I/O interface (§2), so a timer completes on the
same queue as the I/O a worker waits for.

Rulings: project/arch/deps/escape/no-ambient

## 2. I/O interface

The engine has no async runtime dependency. Each worker thread runs
the engine's own event loop over one I/O interface:

```text
trait Io {
  fn submit(&mut self, op: Op) -> Ticket;
  fn poll(&mut self) -> Option<Completion>;
}
struct Worker<I: Io> { io: I, morsels: Queue, ... }
```

The interface has three implementations:

| Backend | Where |
| --- | --- |
| `io_uring` | Linux |
| A pool of threads doing blocking `pread` and `pwrite` | Linux where `io_uring` is unavailable, and other systems with their own direct I/O flag |
| The simulator | `adtb-sim` (§5) |

The simulator controls every thread and every completion through this
interface.

The real runtime is `adtb_sys::os::OsRuntime` (Linux):

```text
OsRuntime::try_new(root, backend) -> Result<OsRuntime, Unavailable>   // paths are relative to root
OsRuntime::new(root, backend) -> OsRuntime                            // panics where try_new errs
enum Backend { Pool { threads }, Uring { entries } }
enum Unavailable { Refused(io::Error), Missing(op) }
```

A caller picks the backend at construction. `Backend::Uring` needs
Linux 5.15 or later; `try_new` refuses it with `Unavailable` when the
host has no io_uring, denies `io_uring_setup` (a sandbox or
`kernel.io_uring_disabled`), or lacks one of the operations it runs.
`Backend::Pool` is never refused. A caller that wants io_uring where
the host allows it tries `Uring` first and falls back to `Pool` on
`Unavailable`; nothing falls back after construction.

Both backends meet every postcondition below and open every file with
the same direct I/O flags, so the same alignment rules hold: where the
file system enforces them, a `Read` or `Write` whose offset or length
is not a multiple of the logical block size completes with `Failed`
and returns its buffer. They differ only in cost:

- `Pool` runs each op as one blocking system call on one of `threads`
  threads, so at most `threads` ops are in the kernel at once.
- `Uring` submits ops to one ring of `entries` slots and completes
  them on one reaping thread; ops beyond `entries` wait in submission
  order for a free slot. `List` has no io_uring operation and runs on a
  one-thread pool; `Close` completes on the submitting thread.

Its threads are host threads, its clock is `CLOCK_MONOTONIC` and its
entropy is `getrandom`. A panic in any of its threads, the backend's
own included, aborts the process. Every file it opens is opened with
`O_DIRECT`, so a file system without direct I/O fails `Open` with
`Failed`.

It injects the faults of §3 into the ops on its files when made with
a fault policy, under the pool backend only:

```text
OsRuntime::with_faults(root, backend, Faults {
  seed, torn_p, corrupt_p, misdirect_p, lost_write_p, latent_p, fail_p,
  lose_unsynced: true, sector, atomic_unit,
}) -> OsRuntime
rt.set_faults(faults)   // from now on; what earlier faults left stays
rt.crash()
```

Each `_p` is the chance of its fault per op that can suffer it, as in
the simulated storage (§5), drawn from `seed` in the order the host
runs the backend's threads, so a seed does not replay a run.

Postconditions of a runtime with faults:

- A failed `Sync` leaves on the device any subset of the writes it
  would have forced, and a later `Sync` of the file succeeds without
  the rest, as ext4, XFS and Btrfs mark pages clean after a failed
  `fsync`.
- `crash` leaves the files as a crash stop does (§3): with
  `lose_unsynced`, any subset of the writes no completed `Sync` covers,
  each whole or, with `torn_p`, in any subset of its atomic units, and
  any prefix of the creations and removals no completed `SyncDir`
  covers. Every op that runs after it completes with `Failed`; a new
  runtime over the directory is the restart.

Data pages and the log are read and written with direct I/O
(`O_DIRECT` on Linux): the page pool is the only cache, and write
errors return from the write itself. A commit still forces the device
cache with `fdatasync`
([storage.md §5.3](storage.md#53-blocks-and-segments)).

On macOS direct I/O is `F_NOCACHE` and the device flush is
`F_FULLFSYNC`; on Windows both come from unbuffered, write-through
handles.

`Runtime::io` returns a new, empty queue; each worker owns one. An op
is one of:

```text
enum Op {
  Open     { path, create }          // -> Opened { file, len }
  Read     { file, offset, buf }     // -> Read(n); n < buf.len() past the end of the file
  Write    { file, offset, buf }     // -> Written; the whole buffer
  Size     { file }                  // -> Size(len)
  Allocate { file, len }             // -> Allocated; the file is at least len bytes, new bytes zero
  Sync     { file }                  // -> Synced; fdatasync
  SyncDir  { dir }                   // -> Synced; fsync of the directory
  List     { dir }                   // -> Listed(paths) of its files, in no particular order
  Remove   { path }                  // -> Removed
  CreateDir { dir }                  // -> Created; also when it exists already
  Close    { file }                  // -> Closed
  Timer    { at }                    // -> Fired, once now() >= at
}
struct Completion { ticket, buf: Option<AlignedBuf>, result: Result<Done, IoError> }
enum IoError { NotFound, Full, Failed }
```

Paths are relative to the database directory, separated by `/`; the
empty path is the database directory itself, which always exists. A
`FileId` names a file opened by this process and stays valid after the
file is removed, until it is closed.

Preconditions:

- The offset and length of a `Read` or `Write` are multiples of the
  device's logical block size, and its buffer is an `AlignedBuf`
  ([sys.md §1](sys.md#1-safe-types)).
- `Close` is submitted with no other op on the file in flight, and the
  `FileId` is never used after it.

Postconditions every backend must meet:

- Every op submitted completes exactly once. Completions arrive in any
  order relative to submission.
- The completion of an op unparks the thread that submitted it (§1),
  so a worker with nothing to run polls its queue and parks.
- A write error is reported in the completion of that write.
- A buffer handed to the backend stays owned by it until its
  completion is polled ([sys.md §2](sys.md#2-unsafe-invariants)); the
  completion returns it.
- `Sync` forces every write to the file that completed before the
  `Sync` was submitted; `SyncDir` forces every creation and removal in
  the directory that completed before it was submitted. Nothing else
  is forced by them.
- `Open` with `create` and `CreateDir` complete with `NotFound` when
  the parent directory is absent; `Open` without `create`, `List` and
  `Remove` do when their path is absent.
- An op that fails because the device or the process's quota has no
  space left completes with `Full`, never `Failed`. A `Write` that
  completes with `Full` may have changed any of its blocks.
- `Size`, like `len` at `Open`, counts every write and allocation to
  the file that completed before the op was submitted.
- `List` returns the full paths of the files in the directory, not its
  subdirectories.
- A crash that loses the creation of a directory loses every file and
  directory under it, whatever forces they had.
- `Close` forces nothing; the file's data and name stay as they were.

TODO: renames are undecided.

Rulings: project/arch/runtime/own-loop, buffer/io-path/direct, buffer/async/uring-fallback, buffer/platform/linux-first

## 3. Faults a backend may produce

The engine is written against this fault model, and every backend may
produce any of it: crash stop, torn writes of any write larger than the
device's atomic unit, bit rot, latent sector errors, misdirected writes
and lost writes. [storage.md §3](storage.md#3-corruption-detection)
states how each is detected.

What each fault leaves behind:

| Fault | Effect |
| --- | --- |
| Crash stop | Every thread stops. Any subset of the writes not covered by a completed `Sync`, including writes still in flight, is on the device afterwards; any prefix of the creations and removals not covered by a completed `SyncDir` is. |
| Torn write | At a crash, an unforced write larger than the atomic unit is on the device in any subset of its atomic units. |
| Bit rot | A stored bit flips; every later read sees it. |
| Latent sector error | Reads of a sector fail with `Failed` until the sector is written again. |
| Misdirected write | The write completes, and its data lands at another offset of the same file; the target keeps its old contents. |
| Lost write | The write completes and never reaches the device, forced or not. |
| Failed write or force | The op completes with `Failed`; a failed write may have changed any of its blocks. |

## 4. Errors

A failed write or `fdatasync` is not retried by the engine; the
process crashes and recovers ([README.md §4](README.md#4-errors)).

## 5. Simulated storage

The whole engine runs inside a simulator that supplies time, I/O,
network, randomness and thread scheduling through the interfaces of
§1 and §2. A deterministic scheduler replaces the operating system's
scheduling of the engine's threads, so morsel workers, the pager and
SSI run multi-threaded under it; a seed replays a run exactly.

The simulated storage is an in-memory disk that injects every fault of
§3:

```text
Sim::new(seed, Config {
  faults: { torn_p, corrupt_p, misdirect_p, lost_write_p, latent_p, fail_p,
            lose_unsynced: true, delay_completions: true },
  sector, atomic_unit, latency, jitter,
})
sim.crash_at(t); sim.run(|rt| ...) -> Finished | Crashed
```

The storage outlives a run, so the run after a crash is a restart of
the process over what the crash left. Each `_p` is the chance of the
fault of §3 per op that can suffer it; `lose_unsynced` off models a
device whose cache survives power loss, `delay_completions` off
completes ops in submission order.

Postconditions of the simulator:

- A seed and a configuration determine every schedule decision,
  completion order, fault, clock value and random draw, so the same
  seed replays a run byte for byte, trace and files alike.
- A run fails when any thread panics, or when every live thread is
  blocked with no completion or timer pending.

After each run, and after each simulated crash and recovery, the
simulator checks that every acknowledged commit is present, that no
unacknowledged state is reported as committed, and that `verify`
finds no structural, cross or logical violation
([../spec/catalog.md §11.1](../spec/catalog.md#111-verification)).
The client records each commit in a ledger outside the process as
submitted, then acknowledged or aborted. A commit still in doubt at a
crash may be present or absent; one aborted, or never submitted, is
reported as committed only by a bug. The reopened database supplies
the commits it reports and the findings of `verify`.

Rulings: buffer/test/dst, buffer/test/dst-threads/sim-scheduler

## 6. Isolation histories

The test harness and the checker crate `adtb-isolation` meet at a
history of list-append transactions
([§7.4](#74-isolation-checking) describes the workload).

The test harness hands the checker a history: a sequence of
`adtb_isolation::Txn` values, whose order carries no meaning.

| Field | Meaning |
| --- | --- |
| `id` | the transaction's number, unique in the history |
| `outcome` | `Committed` when the commit was acknowledged; `Aborted` when the database refused an operation or the commit, or the client aborted; `InDoubt` when the commit was submitted and no answer came |
| `ops` | each `Append { key, elem }` and `Read { key, list }` that returned, in the order the transaction ran them |

Preconditions, which the harness must establish:

- No `(key, elem)` pair is appended twice in one history, counting
  aborted and in-doubt transactions.
- A read's `list` holds the key's elements ordered by `pos`, oldest
  first, the transaction's own appends included.
- A transaction's `ops` holds every operation that returned, and none
  that failed.

Postconditions of `check(history)`:

- It returns `Malformed` when an id or an appended pair repeats, and
  otherwise the anomalies found; an empty list means none was found.
- An `InDoubt` transaction counts as committed exactly when a committed
  transaction read one of its elements, directly or through another
  in-doubt transaction that counts.
- Every anomaly it reports holds in every execution that produces the
  history's observations.
- Each anomaly carries its evidence: a cycle as the list of edges, each
  edge naming its two transactions, its key and the elements that
  order them; a read anomaly names the reader, the writer, the key and
  the element.

| Anomaly | Evidence |
| --- | --- |
| G0 | one cycle of write dependencies per strongly connected component of them |
| G1a | a committed read of an element an aborted transaction appended |
| G1b | a committed read of a list ending in an element its writer followed with another append to the key |
| G1c | one cycle of write and read dependencies per component of them that contains a read dependency |
| G-single | one cycle with exactly one anti-dependency per component of all dependencies |
| G2 | one cycle with more than one anti-dependency, for a component with an anti-dependency and no G-single cycle |
| internal | a read contradicting the transaction's own earlier operations on the key |
| garbage | a read of an element no transaction appended |
| duplicate | a read holding one element twice |
| incompatible order | two committed reads of one key, neither a prefix of the other |

Every kind is a bug under serializability.

The crate also holds scripted scenarios over a `Database` trait (begin,
append, read, commit, abort): write skew, read skew, lost update,
aborted read, intermediate read and circular information flow. No
script appends to a key another open transaction has appended to, so
an engine whose inserts wait on a conflicting writer never blocks one.
A serializable database runs each scenario with no anomaly; for write
skew that means one side aborts.

## 7. Notes for the implementation

Non-binding.

### 7.1 Dependency allowlist

Dependencies come from a short audited allowlist, each entry with a
written justification. The no-ambient rule of §1 is checked by lints:

```toml
# clippy.toml
disallowed-methods = ["std::time::Instant::now", "std::time::SystemTime::now",
                      "std::thread::spawn", "rand::thread_rng"]
```

Dev-dependencies (property testing, Loom, fuzz targets) are
allowlisted separately.

Rulings: project/arch/deps/allowlist

### 7.2 Fault injection under the real engine

A fault-injecting file system runs beneath the real binary and
reproduces how ext4, XFS and Btrfs behave after a failed `fsync`,
including pages marked clean after the failure. It tests the real I/O
backends, which the simulator replaces.

### 7.3 Crash replay

The block-level stream of writes and flushes between the engine and
the device is recorded. For each flush boundary, and each subset of
the writes issued since the last flush, a disk image is built and
recovery runs on it; the checks of §5 must hold for every image.

### 7.4 Isolation checking

Histories of concurrent transactions are checked for Adya's phenomena.
The promise is serializability, so any G0, G1 or G2 cycle in a history
is a bug. The checker is the crate `adtb-isolation`. It follows Elle's
list-append workload: each key holds a list, transactions append
elements unique per key and read whole lists, so every read shows
which appends are visible and in which order they were installed. The
workload stores one row per element at its position in the list:

```adt
type Elem = { id: Key elems, k: I64, pos: I64, e: I64 }
relation elems : Elem
  key id generate sequence
  unique (k, e)
  unique (k, pos)

txn append (k: I64, e: I64) -> Key elems =
  transact
    let n = count(x for x in elems where x.k == k)
    let x = insert elems { k: k, pos: n, e: e }
    return key x
query read_elems (k: I64) -> Set { pos: I64, e: I64 } =
  from x in elems where x.k == k select { pos: x.pos, e: x.e }
```

A client transaction runs a sequence of these appends and reads as one
transaction. Two transactions appending to one key both write position
`n`, so at most one of them commits.

Histories come from two places:

- Simulated clients inside the simulator run registered transactions;
  a cycle comes with the seed that produced it.
- Real server processes are killed, paused and restarted while clients
  drive them over the wire protocol, which finds what the simulation
  does not model.

### 7.5 Memory model

The simulated scheduler interleaves threads at yield points but runs on
the host's memory ordering, so weak-memory reorderings are tested
separately. Loom explores every interleaving of small latch tests (the
page state word, optimistic lock coupling) under the C11 model; Miri
runs the simulated engine and reports data races and undefined
behaviour ([sys.md §4](sys.md#4-notes-for-the-implementation)).

```text
#[cfg(loom)] #[test]
fn olc_reader_sees_consistent_or_restarts() {
  loom::model(|| { /* a writer thread and a reader thread on one page */ });
}
```

### 7.6 The scheduler

Each simulated thread is a host thread, and one baton passes between
them: only the thread holding it runs. Every call into the interfaces
of §1 and §2 except the clock and entropy is a switch point, where the
scheduler advances the clock by a random step, delivers the completions
due, and hands the baton to a random runnable thread. When no thread is
runnable the clock jumps to the next completion. A crash makes every
thread unwind from its switch point.

`Config::switch_rate` adds the switch points of page memory: at each
one a simulated thread passes, the scheduler switches with that
chance. At 0, the default, page memory never switches.

Miri runs these tests as they are, since they are plain threads,
mutexes and the one allocation of `AlignedBuf`; it needs the nightly
toolchain in the dev shell. Loom models the page state word in
`adtb-sys`, not the simulator, and needs `loom` allowlisted as a
dev-dependency. The `io_uring` backend needs `io-uring` or `libc`, and
real entropy needs `getrandom` or `libc`.

### 7.7 Test time

The default `cargo nextest run --workspace` stays under 90 seconds on
16 cores, and each test under 20: nextest marks a test slow at 20
seconds and kills it at 60. Tests build at `opt-level = 1`. A sweep
states its default count of seeds or cases once, as a constant, and
sizes it through `adtb_testkit::profile`, so the `quick` profile runs
one seed and `deep` runs `profile::DEEP` times the default. A new sweep
takes the smallest default count that still exercises its claim, and
builds what does not depend on the seed once per test.

Rulings: buffer/test/fault-fs, buffer/test/crash-replay, project/testing/isolation-check, project/testing/elle/row-per-element, project/testing/where/in-sim, project/testing/where/real-process, project/testing/memory-model/loom, project/testing/memory-model/miri

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
trait Threads { fn spawn(&self, name, body) -> ThreadId; fn spawn_daemon(&self, name, body) -> ThreadId;
                fn current(&self) -> ThreadId;
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
- `spawn_daemon` starts a thread as `spawn` does. Real threads need
  nothing more: a host process ends without them. Only the simulator
  treats a daemon differently.
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
a fault policy, with the same meaning under either backend:

```text
OsRuntime::with_faults(root, backend, Faults {
  seed, torn_p, corrupt_p, misdirect_p, lost_write_p, latent_p, fail_p,
  lose_unsynced: true, no_space, sector: 4096, atomic_unit: 4096,
}) -> OsRuntime
rt.set_faults(faults)   // from now on; what earlier faults left stays; the sector and atomic unit stay
rt.crash()
```

`Faults` is the one fault policy of §3, shared with the simulator
(§5). Each `_p` is the chance of its fault per op that can suffer it,
decided as §3 states from a generator seeded by `seed`, in the order
the host runs the backend's threads, so a seed does not replay a run.
With `no_space`, a write past a file's end and an allocate that grows
a file complete with `Full` and change nothing. Its `sector` is a
multiple of `AlignedBuf::ALIGN`, or `with_faults` panics.

Under either backend the device's geometry, `sector` and
`atomic_unit`, is fixed for the runtime's life: `set_faults` panics
when the faults it is given change either.

Postconditions of a runtime with faults:

- A failed `Sync` leaves on the device any subset of the writes it
  would have forced, and a later `Sync` of the file succeeds without
  the rest, as ext4, XFS and Btrfs mark pages clean after a failed
  `fsync`.
- `crash` leaves the files as a crash stop does (§3): with
  `lose_unsynced`, any subset of the writes no completed `Sync` covers,
  each whole or, with `torn_p`, in any subset of its atomic units, and
  any prefix of the creations and removals no completed `SyncDir`
  covers. Every op admitted after it completes with `Failed`. An op
  the device runs across it completes with `Failed`: a write may land,
  whole or in part, after the crash, and a directory op the device
  finished before the crash may survive it. Dropping the runtime
  returns once every admitted op has completed, so a new runtime over
  the directory, the restart, sees no later write.

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
  CreateNew { path }                 // -> Opened { file, len: 0 }; Exists when the name is taken
  Read     { file, offset, buf }     // -> Read(n); n < buf.len() past the end of the file
  Write    { file, offset, buf }     // -> Written; the whole buffer
  Size     { file }                  // -> Size(len)
  Allocate { file, len }             // -> Allocated; the file is at least len bytes, new bytes zero
  Sync     { file }                  // -> Synced; fdatasync
  SyncDir  { dir }                   // -> Synced; fsync of the directory
  List     { dir }                   // -> Listed(paths) of its entries, in no particular order
  Remove   { path }                  // -> Removed
  RemoveDir { path }                 // -> Removed; the directory is empty
  Rename   { from, to, replace }     // -> Renamed; within a directory or into its parent
  CreateDir { dir }                  // -> Created, or Present when it is a directory already
  Close    { file }                  // -> Closed
  Timer    { at }                    // -> Fired, once now() >= at
}
struct Completion { ticket, buf: Option<AlignedBuf>, result: Result<Done, IoError> }
enum IoError { NotFound, Exists, NotADirectory, Full, Failed }
op.kind() -> OpKind   // the op's variant without its arguments: Open, CreateNew, ..., Timer
```

A `Ticket` is made only by a backend, for an op submitted to it, with
`Ticket::new(n)`; `Ticket::NONE` is the ticket a simulated process that
has stopped answers. `clippy.toml` refuses `Ticket::new` at every other
site, and each backend calls it under an `expect` that
`work/tools/structure-check` counts.

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
- `List` returns the full paths of the entries of any kind in the
  directory, and completes with `NotADirectory` when a component of its
  path is a file.
- `CreateNew` and `CreateDir` complete with `Exists` when an entry of
  another kind has the name, a dangling symbolic link included. `CreateDir`
  completes with `NotADirectory` when a directory above it is a file.
- `Remove` of a directory and `RemoveDir` of a directory holding entries
  complete with `Failed`; `RemoveDir` of a file completes with
  `NotADirectory`, and of an absent path with `NotFound`.
- A crash that loses the creation of a directory loses every file and
  directory under it, whatever forces they had.
- `Close` forces nothing; the file's data and name stay as they were.

- `Rename` moves a file or a directory, with all under it, atomically,
  within one directory or from a directory into that directory's
  parent, on one filesystem. It completes with `Exists` when
  `to` is taken, except that a directory replaces an empty directory
  and, with `replace`, a file replaces a file. It completes with
  `NotFound` when `from` is absent and `NotADirectory` when a directory
  above them is a file. A `FileId` stays valid across it.
- A `Rename` is forced once `SyncDir` has completed after it on the
  directory of `from` and on the directory of `to`. Until then a crash
  keeps it whole or loses it whole, so the entry is at exactly one of
  the names. `SyncDir` of an absent
  directory completes with `NotFound`.
- `Open`, `CreateNew`, `Remove` and `RemoveDir` complete with
  `NotADirectory` when a directory above their path is a file.

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

A backend with faults decides the fate of each op from one
`adtb_sys::runtime::Faults` and a source of draws, by the decision
function of the op's kind, and applies the fate; it draws nothing for
an op itself. A fault whose chance is zero draws nothing.

```text
decide_write(draw, &faults, offset, len, file_len) -> WriteFate { Full, Fail { landed }, Lost, At(target) }
decide_read(draw, &faults, offset, n, unreadable) -> ReadFate { Latent { sector }, Unreadable, Corrupt { byte, bit }, Clean }
decide_allocate(draw, &faults, len, file_len) -> AllocateFate { Fail, Full, Run }
decide_force(draw, &faults) -> ForceFate { Fail, Run }   // Sync and SyncDir
decide_unforced(draw, &faults, len, completed) -> Landing { Whole, Lost, Torn(units) }   // at a crash
trait Draw { chance(p), below(n), coin() }
```

Postconditions, in the order of the draws:

- A write: with `no_space`, a write past the file's end is `Full` and
  draws nothing. Otherwise it draws fail, and for a failed write one
  coin per sector of it, the sectors that land; then lost; then
  misdirect, and for a misdirected write a uniform pick among the other
  sector-aligned offsets where it fits within the file, or just past
  the file's end when there is none. `At(offset)` is the write as asked.
- A read that finds `n` bytes: none is `Clean` and draws nothing.
  Otherwise it draws latent, and its sector uniformly among those of
  the bytes; then, unless one of those sectors is unreadable already,
  corrupt, its byte uniformly among the `n` and its bit uniformly among
  eight.
- An allocate draws fail; then with `no_space` one that grows the file
  is `Full`.
- A `Sync` or `SyncDir` draws fail.
- At a crash, each write no force covers, in the backend's order: a
  completed write without `lose_unsynced` is `Whole` and draws
  nothing. Otherwise a write larger than the atomic unit draws torn,
  and when it strikes one coin per atomic unit, the units that land;
  a write that is not torn draws one coin, landing it whole or losing
  it.

## 4. Errors

A failed write or `fdatasync` is not retried by the engine; the
process crashes and recovers ([README.md §4](README.md#4-errors)).

Storage raises fatal I/O as an `adtb_sys::runtime::FatalIo` panic payload
carrying the original `IoError` and diagnostic operation context. An
engine catch must resume that payload. Simulation helpers recognize it
only outside `Sim::run`, after the simulator has applied its crash
lifecycle; they resume every other payload. A lookalike string is an
ordinary panic, not evidence of an I/O failure.

The raising point emits a best-effort diagnostic before unwinding. A
failure to write that diagnostic does not replace the typed payload.
Suppressing the panic hook does not suppress this direct diagnostic.

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
  faults: Faults { torn_p, corrupt_p, misdirect_p, lost_write_p, latent_p, fail_p,
                   lose_unsynced: true, no_space, sector: 512, atomic_unit: 512 },
  delay_completions: true, latency, jitter,
})
sim.set_faults(faults)   // from now on; the sector and atomic unit stay
sim.crash_at(t); sim.run(|rt| ...) -> Finished | Crashed
rt.crash_after(d)        // inside a run: crash it at now + d, in place of sim.crash_at
```

A plan acts on chosen ops, from inside a run or between runs:

```text
sim.plan(plan); rt.plan(plan)    // in place of the plan before
rt.extend(plan)                  // added to the plan, its rules after
sim.own_plan(owner, plan)        // and refuse every later plan
Plan::new().rule(ops: Match, action).first(ops, action).at(n, action)
  .lasting()                     // its rules outlive the run
Match::any() | Match::kind(k) | Match::kinds(ks) | Match::any_of(ms)
  .path(p) | .prefixed(p) | .suffixed(p) | .containing(p)
                                 // the path an op names
  .from_containing(p)            // a rename's source path
  .offset(o) | .covers(o) | .whole(unit)   // a read's or write's range
  .creating()                    // an open that creates
  .except(m)
Action::Fail(IoError) | Action::Lose | Action::Run(closure) | Action::Hold
rt.arm(ops: Match); rt.disarm() -> u64
sim.numbered(), rt.numbered() -> Numbered { kinds, struck, armed_at }
sim.counted(ops), rt.counted(ops: Match) -> Count  // count.get()
sim.logged(ops), rt.logged(ops: Match) -> OpLog    // log.entries()
Submitted { ticket, at, completed, kind, path, from, offset, length,
            bytes, recorded, planned }
rt.files_open() -> usize; rt.files() -> files as a reader sees them
```

The path an op names is the path its file was opened at, a rename's
new name, and otherwise its own path or directory. A match holds no
code: every test it makes is one of the forms above.

Postconditions of a plan:

- The first rule that matches an op acts on it; a `first` rule acts on
  one op and is then dropped. While armed, an op no rule acted on and
  that the armed match accepts gets the next number, counting on from
  earlier ops, and the action at that number is taken; `disarm` drops
  the numbered actions not taken and returns how many ops were
  numbered. `struck` is the first numbered op an action was taken at.
- The rules end with the run that set them, even one that panics,
  unless planned `lasting`; those last through the simulation's later
  processes. Numbering, numbered actions,
  counts and logs last across the simulation's processes, so a search
  numbers the ops of a run and its restarts as one sequence.
- `Fail(e)` completes the op with `e`, and `Lose` completes a write as
  `Written`; neither changes the device, and a crash lands nothing of
  such an op still queued. A planned op completes through the
  simulator's one completion path, so its completion unparks its
  submitter (§2).
- `Run` runs its closure on the submitting thread before the op is
  queued; the op then runs as any other. A `Run` is planned with
  `first` or `at`; a `rule` of one panics.
- A `Fail` that may match a `Close` panics when planned, and a numbered
  `Fail` that comes to a `Close` panics there: a failed close would
  leave its file open.
- A `Lose` whose match may take an op other than a write panics when
  planned.
- A plan after `own_plan` panics, naming the owner.
- A held op never completes. It is pending, so the deadlock rule does
  not fire; the clock skips it when choosing the next due event; a crash
  takes it as an op in flight.
- A count counts each op its match accepts, at submission. A log holds
  each such op with its ticket, the time it was submitted and the time
  it completed, its path and range, a write's bytes, the device events
  recorded before it and the action planned for it. The simulator's
  tickets are one sequence across its queues, in submission order, so
  a later ticket is a later submission.
- Matching, numbering, counting and logging draw nothing from the seed,
  so a run with no plan replays as it did before plans.

Limit: when every thread waits and an op is held, the clock moves to a
scheduled crash. A run that waits on a held op and on nothing else
then ends as a crash, where without the crash it would fail.

The real backend has no plans.

The storage outlives a run, so the run after a crash is a restart of
the process over what the crash left. `Faults` is the policy of §3,
its fates decided as §3 states from the simulator's seed; its own
`seed` is unused. Its `sector` is the device's logical block, dividing
`AlignedBuf::ALIGN`, and it and `atomic_unit` are fixed for the
runtime's life, as under the real backend (§2). `lose_unsynced` off
models a device whose cache survives power loss; `delay_completions`
off completes ops in submission order.

Postconditions of the simulator:

- A seed and a configuration determine every schedule decision,
  completion order, fault, clock value and random draw, so the same
  seed replays a run byte for byte, trace and files alike.
- A run fails when any thread panics, or when a live thread that is no
  daemon is blocked with no completion pending and only a daemon's timer
  left. A run whose threads return with an op held fails, whether or
  not a crash is scheduled. A run where no thread can run while an op
  is held fails when no crash is scheduled; with a crash scheduled, the
  clock moves to it (the limit in §5).
- A run ends when every thread that is no daemon has returned and every
  daemon is idle: blocked, with only daemons' timers left. Those timers
  do not fire and do not move the clock.
- A daemon's timer is nothing a thread that is no daemon may wait on: a
  non-daemon joining a daemon on a timer, or parked for a daemon to wake
  it after one, fails the run as a deadlock.
- A daemon that stays runnable hides a deadlock and keeps the run going.
- A thread a daemon spawns is no daemon.

After the crash and recovery of every round, the harness's crash loop
(`adtb_harness::crash::crash_loop`, [testing.md §5](testing.md#5-harness))
checks with `adtb_sim::check` that every acknowledged commit is
present, that no unacknowledged state is reported as committed, and
that `verify` finds no structural, cross or logical violation
([../spec/catalog.md §11.1](../spec/catalog.md#111-verification)).
The reopened database's `allows` filters the result: a violation it
allows is returned with the round, and every other one fails it. The
client records each commit in a ledger outside the process as
submitted, then acknowledged or aborted.
A commit still in doubt at a crash may be present or absent; one
aborted, or never submitted, is reported as committed only by a bug.
The reopened database supplies the commits it reports and the findings
of `verify`.

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

The simulator's tests run both on the engine's public write path,
through `adtb_simkit::elle::Elems`: an `adtb::Database` on the
simulator opened on §7.4's schema with `append` and `read_elems`
registered, and `Client`, its implementation of the `Database` trait.

```text
Elems::open(rt, &OpenOptions) -> Result<Elems, OpenError>   // options: elle::options()
Elems::{begin(Kind) -> Result<Txn, Error>, append(&mut Txn, k, e) -> Result<(), Error>,
        read(&mut Txn, k) -> Result<Vec<i64>, Error>, lists(keys) -> Vec<Vec<i64>>,
        verify() -> Vec<String>, database()}
Kind::{ReadWrite, ReadOnly, Deferrable}; Client(&Elems): adtb_isolation::Database
elle::aborted(Error) -> Aborted
elle::created() -> BTreeMap<String, Vec<u8>>
elle::simulate_with(seed, &OpenOptions, body: FnOnce(&SimRuntime, &Arc<Elems>) -> T) -> T
elle::simulate(seed, body) -> T
```

Preconditions:

- `simulate_with`'s options create the file `elle::options()` creates.

Postconditions:

- `begin(kind)` begins an interactive transaction: `ReadOnly` as
  read-only, `Deferrable` as read-only and deferrable, `ReadWrite` as
  neither. `Client::begin` begins a `ReadWrite` one.
- `append(k, e)` calls the registered `append` in the transaction;
  `read(k)` calls `read_elems` and returns the elements ordered by
  `pos`, the transaction's own appends included. Each returns the
  engine's error as it is.
- `aborted` maps a serialization failure and a violation of
  `unique (k, pos)` to `Aborted` and panics at any other error.
  `Client`'s operations and its commit fail through it, so two appends
  at one position of a list leave at most one of them committed.
- `created` returns the files of a new workload database as
  `elle::options()` creates it, built once per process.
- `simulate_with` runs `body` in a simulated process of `seed` over
  those files, opened with the options, then panics when `verify`
  finds anything; `simulate` does so with `elle::options()`.

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
separately. Loom explores the interleavings of small models of the page
state word under the C11 model: `adtb-sys`'s unit tests built with
`--cfg adtb_loom`, which `loom` in the dev shell lints and runs. Each
model is two threads on one page: latches against latches, optimistic
reads and upgrades against writers, and the replacement clock against
both. Plain reads of page bytes under a latch are checked through a
cell beside each word that records them. Miri runs the unit tests of
`adtb-sys` and reports data races and undefined behaviour
([sys.md §4](sys.md#4-notes-for-the-implementation)).

```text
#[cfg(all(test, adtb_loom))] #[test]
fn an_optimistic_read_sees_a_version_change_or_its_versions_page() {
  loom::model(|| { /* a writer thread and a reader thread on one page */ });
}
```

### 7.6 The scheduler

Each simulated thread is a host thread, and one baton passes between
them: only the thread holding it runs. Every call into the interfaces
of §1 and §2 except the clock and entropy is a switch point, where the
scheduler advances the clock by a random step, delivers the completions
due, and hands the baton to a random runnable thread. When no thread is
runnable the clock jumps to the next completion; a held op is never
due, so the clock passes it for the next completion, timer or crash. A
crash makes every
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
the development machine, and each test under 20: nextest marks a test slow at 20
seconds and kills it at 60. Tests build at `opt-level = 1`. A sweep
states its default count of seeds or cases once, as a constant, and
sizes it through `adtb_testkit::profile`, so the `quick` profile runs
one seed and `deep` runs `profile::DEEP` times the default. A new sweep
takes the smallest default count that still exercises its claim, and
builds what does not depend on the seed once per test.

Rulings: buffer/test/fault-fs, buffer/test/crash-replay, project/testing/isolation-check, project/testing/elle/row-per-element, project/testing/where/in-sim, project/testing/where/real-process, project/testing/memory-model/loom, project/testing/memory-model/miri

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

Data pages and the log are read and written with direct I/O
(`O_DIRECT` on Linux): the page pool is the only cache, and write
errors return from the write itself. A commit still forces the device
cache with `fdatasync`
([storage.md §5.3](storage.md#53-blocks-and-segments)).

On macOS direct I/O is `F_NOCACHE` and the device flush is
`F_FULLFSYNC`; on Windows both come from unbuffered, write-through
handles.

Postconditions every backend must meet:

- A write error is reported in the completion of that write.
- A buffer handed to the backend stays owned by it until its
  completion is polled ([sys.md §2](sys.md#2-unsafe-invariants)).

Rulings: project/arch/runtime/own-loop, buffer/io-path/direct, buffer/async/uring-fallback, buffer/platform/linux-first

## 3. Faults a backend may produce

The engine is written against this fault model, and every backend may
produce any of it: crash stop, torn writes of any write larger than the
device's atomic unit, bit rot, latent sector errors, misdirected writes
and lost writes. [storage.md §3](storage.md#3-corruption-detection)
states how each is detected.

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
([../spec/catalog.md §11.1](../spec/catalog.md#111-verification)).

Rulings: buffer/test/dst, buffer/test/dst-threads/sim-scheduler

## 6. Notes for the implementation

Non-binding.

### 6.1 Dependency allowlist

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

### 6.2 Fault injection under the real engine

A fault-injecting file system runs beneath the real binary and
reproduces how ext4, XFS and Btrfs behave after a failed `fsync`,
including pages marked clean after the failure. It tests the real I/O
backends, which the simulator replaces.

### 6.3 Crash replay

The block-level stream of writes and flushes between the engine and
the device is recorded. For each flush boundary, and each subset of
the writes issued since the last flush, a disk image is built and
recovery runs on it; the checks of §5 must hold for every image.

### 6.4 Isolation checking

Histories of concurrent transactions are checked for Adya's phenomena.
The promise is serializability, so any G0, G1 or G2 cycle in a history
is a bug. The workload appends one row per element, so every read
shows which appends are visible:

```adt
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

### 6.5 Memory model

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

Rulings: buffer/test/fault-fs, buffer/test/crash-replay, project/testing/isolation-check, project/testing/elle/row-per-element, project/testing/where/in-sim, project/testing/where/real-process, project/testing/memory-model/loom, project/testing/memory-model/miri

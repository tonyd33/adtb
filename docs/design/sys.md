# adtb-sys

`adtb-sys` holds every `unsafe` block of the implementation: page
memory, direct I/O buffers, `io_uring`, atomic page reads and the
CRC-32C instruction. It exposes safe types to `adtb`. Every other crate has
`#![forbid(unsafe_code)]`.

## 1. Safe types

The safe types are `Frame`, `PageRef`, `AlignedBuf`, `Lock` and `RwLock`.

`adtb-sys` states its contract in its documentation; this file states
what `adtb` may rely on.

`AlignedBuf` is an owned, zero-initialised byte buffer for direct I/O,
read and written as a `[u8]`:

```text
AlignedBuf::zeroed(len) -> AlignedBuf     // panics if len is 0
AlignedBuf::from_slice(&[u8]) -> AlignedBuf
AlignedBuf::ALIGN = 4096
```

Invariants:

- Its address is a multiple of `AlignedBuf::ALIGN`, which every
  supported device's logical block size divides.
- Its length is fixed when it is allocated and is never zero.
- It is `Send` and `Sync`, so an op may carry it to another thread.

`adtb-sys` also defines the runtime interfaces of
[runtime.md §1](runtime.md#1-time-threads-and-randomness) and
[§2](runtime.md#2-io-interface) as its module `runtime`. Its module
`os` holds the real runtime over the host
([runtime.md §2](runtime.md#2-io-interface)).

Page memory is `pages::PageMemory`, with three guards in place of
`Frame` and `PageRef`:

```text
PageMemory::reserve(page_size, pages) -> PageMemory    // every page evicted
mem.lock_exclusive(pid, threads) -> Exclusive          // waits; latches an evicted page too
mem.try_lock_exclusive(pid) -> Option<Exclusive>       // resident and unlatched only
mem.lock_shared(pid, threads) -> Option<Shared>        // None if evicted
mem.optimistic(pid) -> Option<Optimistic>              // None if exclusive or evicted
mem.mark(pid) -> bool                                  // the clock's mark; true if marked already
exclusive.resident() / exclusive.write(offset, bytes) / exclusive.evict() / exclusive.share()
optimistic.read(offset, out) / optimistic.validate() / optimistic.upgrade()
```

`Exclusive` and `Shared` read as `&[u8]`. Waiting calls
`threads.yield_now`. Only `adtb`'s pager uses these types; consumers
see pages as [storage.md §2.3](storage.md#23-page-access) states.

Page memory has switch points: before `lock_exclusive`,
`try_lock_exclusive`, `lock_shared`, `optimistic`, `upgrade` and
`validate`, and halfway through `optimistic.read`.
`pages::switch_points(hook)` calls the hook at each one the calling
thread passes until the returned guard drops; the simulator installs
one that may switch threads or unwind on a crash.

Preconditions:

- The caller leaves no in-memory structure half-updated across a page
  memory call, as for a call into the runtime
  ([runtime.md §1](runtime.md#1-time-threads-and-randomness)).

`lock::Lock<T>` is the mutex every in-memory structure of `adtb`
shared between threads sits behind:

```text
Lock::new(value) -> Lock<T>
lock.lock(threads) -> MutexGuard<T>     // waits through threads.yield_now
RwLock::new(value) -> RwLock<T>
rw.read(threads) -> RwLockReadGuard<T>  // shared with other readers
rw.write(threads) -> RwLockWriteGuard<T>
```

`lock::RwLock<T>` waits as `Lock` does and follows the same rules.

Invariants:

- A waiter spins only through `yield_now`, so a scheduler that switches
  threads only at runtime calls and page memory switch points runs the
  holder.
- A lock whose holder panicked is handed to the next caller as it was
  left; it is never refused.

Preconditions:

- A holder may block in the runtime, as for I/O, but never parks
  waiting for a thread that needs the lock.
- A thread never asks for a lock it holds.

`crc32c::update(register, bytes) -> u32` advances a CRC-32C (Castagnoli,
reflected polynomial `0x82F63B78`) register over `bytes`, starting from
`!0`, the caller inverting the last value. Its result depends only on
its arguments: whichever of the instruction and the table the CPU allows
gives the same value, so a file is valid on any CPU.

Rulings: project/arch/unsafe/confined

## 2. Unsafe invariants

`adtb-sys` guarantees, and `adtb` may rely on:

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
- A file descriptor handed to the kernel belongs to a file that stays
  open until the call returns: an op holds its file while it runs, so a
  `Close` releases the descriptor only after the last op on it.
- Memory the kernel writes into outside I/O, a clock reading or random
  bytes, is a live, writable buffer of the length the call is given.

## 3. Page state word

Each page has a 64-bit state word:

```text
state[p]
  bits 56..63  0 unlocked | 1..252 shared count | 253 exclusive | 254 marked | 255 evicted
  bits  0..55  version, incremented on every exclusive unlock and on eviction
```

Optimistic readers read the version, read the page, and revalidate the
version. A reader may rely on an unchanged version and an unlocked word
meaning no writer changed the page between the two reads.

A page in flight for I/O is held exclusive until its completion is
processed, so cancellation of the requesting work never frees memory
the kernel writes into.

Rulings: buffer/cache/vmcache

## 4. Notes for the implementation

Non-binding.

Miri runs the unit tests of `adtb-sys`: page memory and its latches,
aligned buffers, CRC-32C, and the host runtime's thread bookkeeping.
Under Miri page memory is heap memory in place of the anonymous mapping,
and `crc32c::update` takes the table, by `cfg(miri)`, in place of the
instruction. The two
tests that install a seccomp filter to deny `io_uring_setup` are
ignored, since Miri runs neither. Nothing yet runs the engine over the
simulated storage of [runtime.md §5](runtime.md#5-simulated-storage)
under Miri, and nothing runs the real I/O backends under
AddressSanitizer or ThreadSanitizer.

```sh
$ miri            # in the dev shell, on its nightly toolchain
$ miri pages::    # only the tests matching a filter
```

Rulings: project/arch/unsafe/miri-io/sim-storage, project/arch/unsafe/miri-io/sys-tests

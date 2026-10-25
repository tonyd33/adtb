# adtb-sys

`adtb-sys` holds every `unsafe` block of the implementation: page
memory, direct I/O buffers, `io_uring` and atomic page reads. It
exposes safe types to `adtb`. Every other crate has
`#![forbid(unsafe_code)]`.

## 1. Safe types

The safe types are `Frame`, `PageRef` and `AlignedBuf`.

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
[§2](runtime.md#2-io-interface) as its module `runtime`.

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

Miri runs the whole engine over the simulated storage of
[runtime.md §5](runtime.md#5-simulated-storage), which is plain Rust,
through the tests of `adtb-sim`, so the pager, latches and page layouts
are checked for undefined behaviour and data races. The real I/O
backends, which Miri cannot execute, are tested natively under
AddressSanitizer and ThreadSanitizer.

```sh
$ cargo miri test -p adtb-sim
```

Rulings: project/arch/unsafe/miri-io/sim-storage, project/arch/unsafe/miri-io/sys-tests

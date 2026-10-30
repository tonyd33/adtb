# Rust

kind: language
home: https://doc.rust-lang.org/reference/

## non_exhaustive

- Outside the defining crate, matches on a `#[non_exhaustive]` enum need `_`; no effect inside the crate; blocks variant construction outside. [V 2026-10-25] https://doc.rust-lang.org/reference/attributes/type_system.html

## Async I/O runtimes

- glommio: cooperative thread-per-core runtime on io_uring, Linux only. [V 2026-10-25] https://github.com/DataDog/glommio
- monoio: thread-per-core, io_uring or epoll on Linux (io_uring needs 5.6+ and memlock limits), kqueue on macOS; tasks need not be Send or Sync. [V 2026-10-25] https://github.com/bytedance/monoio
- tokio-uring: runs on top of tokio; buffers are passed by ownership into an operation and returned with the result. [V 2026-10-25] https://github.com/tokio-rs/tokio-uring

## Concurrency and UB testing tools

- loom: runs a test many times permuting concurrent executions under the C11 memory model, with state reduction (CDSChecker). Enabled via cfg(loom). [V 2026-10-25] https://github.com/tokio-rs/loom
- shuttle (AWS): randomized concurrency testing (PCT scheduler), controls thread scheduling, reproduces failing schedules deterministically; wrappers for tokio, rand, std::sync. [V 2026-10-25] https://github.com/awslabs/shuttle
- Miri: interpreter detecting UB in unsafe code: out of bounds, use-after-free, uninitialised data, misalignment, invalid values, data races and some weak-memory effects; Stacked and Tree Borrows experimental. [V 2026-10-25] https://github.com/rust-lang/miri

## MSRV practice

- tokio: rolling MSRV of at least 6 months; MSRV raised only in minor releases; LTS minor releases. [V 2026-10-25] https://github.com/tokio-rs/tokio/blob/master/README.md

## Numbers

- RFC 560: overflow of fixed-size integer arithmetic is a program error, not undefined behaviour; checked (panics) in debug builds, may wrap in release; wrapping_*, checked_*, saturating_* methods and Wrapping<T> for intended wrap. [V 2026-10-25] https://rust-lang.github.io/rfcs/0560-integer-overflow.html
- f64::total_cmp is IEEE 754-2008 totalOrder: negative NaNs, -inf, negatives, -0.0, +0.0, positives, +inf, positive NaNs; differs from PartialEq where NaN != NaN. [V 2026-10-25] https://doc.rust-lang.org/std/primitive.f64.html

## Loom and Miri in use

- loom 0.7.2 runs each model thread as a coroutine of the generator crate on the test's OS thread, so a std `thread_local!` is shared by every model thread; `loom::thread_local!` gives each its own. [V 2026-10-29] loom-0.7.2/src/rt/scheduler.rs
- loom 0.7.2's `thread_local!` matches its initializer as an edition-2018 `$init:expr`, which refuses `const { ... }`; strip the `const` before handing it the declaration. [V 2026-10-29] loom-0.7.2/src/lib.rs
- loom 0.7.2 models `fence` with Acquire, Release, AcqRel and SeqCst; a Relaxed fence panics. Its atomics are tracked objects, not plain memory, so a buffer reinterpreted as atomics needs a separate model of the buffer. [V 2026-10-29] loom-0.7.2/src/rt/atomic.rs
- loom 0.7.2 pulls in generator, scoped-tls, tracing and tracing-subscriber (env-filter, so matchers and regex-automata): 19 crates new to a lockfile with only libc and io-uring before. [V 2026-10-29] cargo tree
- rust-overlay's `selectLatestNightlyWith (t: t.minimal.override { extensions = [ "miri" "rust-src" ]; })` picks the newest nightly that ships Miri; `cargo miri test` then prepares its sysroot on the first run with no further setup. [V 2026-10-29] github:oxalica/rust-overlay afcefd09
- Miri cannot run `prctl` (seccomp) or `io_uring_setup`: "unsupported operation: Miri does not support `prctl` syscall with op=38". Thread spawn, park and unpark run. `-Zmiri-many-seeds=0..N` reruns each test under N schedules. [V 2026-10-29] cargo miri on nightly-2026-10-29
- loom 0.7.2 did not find a stale read that C11 allows: with the optimistic reader's first load of the state word made Relaxed, a model where a writer stores its data Relaxed under an exclusive latch and unlatches with a Release store passed, and a validated copy at the new version could still hold the old bytes. The same mutation fails a model where the writer hands over through a shared latch. Loom's README lists executions it does not explore under "Unsupported features" (load buffering), so a passing model is not proof for Relaxed loads. [V 2026-10-29] mutation runs of crates/adtb-sys/src/pages/loom_tests.rs; loom-0.7.2/README.md
- loom 0.7.2's `loom::cell::UnsafeCell<()>` beside an atomic word records plain accesses: `with` for a read, `with_mut` for a store. It panics "Causality violation: Concurrent read and write accesses" when a read is not ordered with a store, which models plain reads of memory that is also accessed atomically. [V 2026-10-29] loom-0.7.2/src/rt/location.rs

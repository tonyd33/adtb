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

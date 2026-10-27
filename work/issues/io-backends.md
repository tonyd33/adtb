---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 310
summary: adtb-sys has no io_uring or blocking-pool backend for the Io trait
closed: 2026-10-27
tags:
  - sim
  - engine
---

# Feature: io_uring and blocking-pool backends

## Report

Milestone 3 of `docs/plan.md` needs two real backends for the `Io`
trait in `adtb-sys`: `io_uring` and a blocking thread pool. Neither
exists. Both need a dependency approved in `deny.toml`, `io-uring` or
`libc`.

## 2026-10-25: interface gaps

The pager found that the `Io` interface has no directory creation, file
size query or close. The pager re-opens a file to learn its length, and
the log directory exists only implicitly. That is fine in the
simulator, but a real backend needs all three. State them in
`docs/design/runtime.md` and add them to both backends.

## 2026-10-25: pool backend landed

The blocking-pool backend and `Size`, `CreateDir` and `Close` landed with `real-runtime`; the interface gaps are closed. Still open: io_uring. My ruling: `bitflags` is approved for it, so use the `io-uring` crate (0.7, with `bitflags` 2 and `cfg-if`) rather than hand-written rings. Add it as a `Backend` variant dispatched in `OsIo::submit`, reusing the open-file table in `os/files.rs`, and run it through the `contract!` tests in `io_contract.rs`.

## 2026-10-27: done

The io_uring backend landed in `adtb-sys` as `Backend::Uring { entries }`, on the `io-uring` crate 0.7 (with `bitflags` 2 and `cfg-if`, allowlisted in `deny.toml`). It runs `Open`, `Read`, `Write`, `Size`, `Allocate`, `Sync`, `SyncDir`, `Remove` and `CreateDir` as ring operations, `List` on a one-thread pool, and `Close` on the submitting thread, over the open-file table of `os/files.rs`. `OsRuntime::try_new` reports `Unavailable` when the host refuses io_uring or lacks an operation, so a caller falls back to `Backend::Pool`; `docs/design/runtime.md` states how a caller picks and what each backend guarantees. The `contract!` tests run against it, with an unaligned-request test on both real backends and a seccomp-denied `io_uring_setup` test.

Still open: fault injection runs only beneath the pool backend (`with_faults` refuses `Uring`), and `Database::open` always picks the pool.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the iou track: `Backend::Uring` in `crates/adtb-sys/src/os/uring.rs`, run through the `contract!` tests. Follow-ups are in `uring-follow-ups`.

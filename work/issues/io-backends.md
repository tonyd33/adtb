---
created-at: 2026-10-25
status: open
kind: feature
priority: 310
summary: "adtb-sys has no io_uring or blocking-pool backend for the Io trait"
tags: [sim, engine]
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

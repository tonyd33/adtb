---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 330
summary: the real runtime has no fault-injecting file system
closed: 2026-10-26
tags:
  - sim
---

# Feature: fault injection under the real runtime

## Report

`docs/design/runtime.md` 7.2 describes faults injected under the real engine. `OsRuntime` has none yet.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mpwvomok: an in-process injector under `OsRuntime::with_faults`. A file system outside the process, which `docs/design/runtime.md` 7.2 describes, does not exist; a seed does not replay a run, since the I/O threads interleave.

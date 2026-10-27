---
created-at: 2026-10-27
status: open
kind: feature
priority: 863
summary: "a stopped scan still holds the heap-wide SIREAD lock, the batch being filled sits outside the grant, and a full spill device is reported as Fatal Storage"
tags:
  - engine
  - quality
---

# Feature: scan and spill follow-ups

## Report

- `Store::pages` takes the heap-wide SIREAD lock before the first page, so an early-stopped scan still conflicts with every writer of the heap. Take it per page read, or escalate as `docs/design/mvcc.md` states.
- The batch a scan is filling is held outside the memory grant.
- `catalog.md` section 8.1 reports a call with no spill space to spare as `StatementLimit Memory`, but the runtime's `IoError` is only `NotFound` or `Failed`, so a full spill device comes back as `Fatal Storage`. Ruling: add a full-device kind to `IoError` in `docs/design/runtime.md`, map ENOSPC to it in both backends and the simulator, and report a spill that meets it as `StatementLimit Memory`. Other spill I/O errors stay `Fatal Storage`.
- `HeapNo` is used by `Heap::id`, `Link::heap`, `walk` and the `Reads` trait; `Heap::create` and `open`, the serializer, watermarks, vacuum's dead set and progress, and `Scope::Heaps` still take `u32`.

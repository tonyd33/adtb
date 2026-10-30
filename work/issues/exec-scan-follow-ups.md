---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 863
summary: "a stopped scan still holds the heap-wide SIREAD lock, the batch being filled sits outside the grant, and a full spill device is reported as Fatal Storage"
closed: 2026-10-29
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

## 2026-10-27: done (full-device item only)

`IoError::Full` is in `docs/design/runtime.md` and `adtb-sys`. The pool backend maps `StorageFull` and `QuotaExceeded` to it, io_uring maps `ENOSPC` and `EDQUOT`, and the simulator's `Faults::no_space` refuses writes past a file's end and allocates that grow a file. A spill that meets `Full` is `StatementLimit Memory` of the grant. Tests: `exec::a_spill_onto_a_full_device_is_rejected_for_its_memory_grant`, `faults::a_device_with_no_space_refuses_growth_and_keeps_overwrites`, `adtb-sys os::tests::both_backends_report_a_device_with_no_space_as_full`. The other three items are still open.

## 2026-10-27: full device

The full-device item landed with the q799 track: `IoError::Full`, mapped from ENOSPC and EDQUOT in both backends and the simulator, reported for a spill as `StatementLimit Memory`. The other items stay open.

## 2026-10-29: done

A scan holds a scan lock from before its page listing. It becomes the heap lock when the scan ends, or page locks on what it read when one row decided an `exists` or `is_empty` (`docs/design/mvcc.md` 8.2). The filling batch is charged to the grant. Landed at ea40a463. The `HeapNo` item moved to heap-no-rest.

## 2026-10-29: resolved

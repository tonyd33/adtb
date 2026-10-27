---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 895
summary: "a vacuum whose history heap write fails on a page error is reported as Internal with Debug text, not as a storage fault"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# A history heap write failure in vacuum is reported as Internal with Debug text

## Report

`crates/adtb/src/write/mod.rs:707` maps `vacuum::Error::Heap(h)` to `Error::Internal(format!("vacuum wrote history: {h:?}"))`. `heap::Error` (`crates/adtb/src/heap/handle.rs:28-36`) is `Page(PageError)` or `TooLarge`, so an I/O failure or a full device while vacuum copies a version into a history heap reaches the API as an internal error with flattened text instead of a storage fault.

Fix: map `heap::Error::Page(p)` to `Error::Page(p)`, as the arm above does for `vacuum::Error::Page`; keep `TooLarge` as an invariant breach whose text names the invariant.

## 2026-10-27: done

`write::Error` has `From<vacuum::Error>`: `Heap(Page(p))` maps to `Error::Page(p)`, and `Heap(TooLarge)` to `Internal` with text naming the invariant. `vacuum` uses it through `?`. A test in `crates/adtb-sim/tests/history.rs` covers the conversion; no test forces a page fault into a history write through the API.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qhist track.

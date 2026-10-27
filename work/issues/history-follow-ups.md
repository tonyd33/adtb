---
created-at: 2026-10-27
status: open
kind: feature
priority: 876
summary: "safe-snapshot markers logged after the last durable commit are lost at a crash, raising the window from zero keeps nothing deleted before, and at blocks are not typed at their pinned version"
tags:
  - engine
  - lang
---

# Feature: reads of the past follow-ups

## Report

- A safe-snapshot marker logged after the last durable commit is lost in a crash: recovery does not rebuild markers from the log, so a read at that position reads an earlier marker.
- Raising the retention window from zero does not keep versions deleted while it was zero, since their commit times were never recorded. The ruling says the change applies to versions vacuum has not reclaimed; state which in `catalog.md` section 10.2 or record commit times always.
- An `at` body is typed at the version the program is checked at, and the run-time version check catches only rewritten heaps: a relation or type renamed without a rewrite, or a redefined function the block calls, is not detected. Type the body at the pinned version as `derived.md` section 6.3 states.
- The hash in `pins` is not compared with the log's.
- `sys::commit_times` and `sys::safe_snapshots` are not filled.
- `catalog::settings` holds no default rows, though the spec says it holds one row of each constructor.
- Every relation, engine relations included, gets history structures at create, whatever the window.
- The reference interpreter refuses `at`, so no conformance file covers reads of the past.

## 2026-10-27: done (first item only)

A marker is now durable before a read uses it. `log_safe_snapshot` records the log end after keeping a marker, under a lock it holds while keeping one, and `pin` and `pin_time` force the log to that end before they return a marker. The new `Pager::appended` and `Pager::flush_below` support this. `a_marker_a_read_of_the_past_used_survives_a_crash` in `crates/adtb-sim/tests/history.rs` failed before the fix. The other items remain open.

## 2026-10-27: markers

The first item landed with the qhist track: a marker is durable before a read uses it. The rest stays open.

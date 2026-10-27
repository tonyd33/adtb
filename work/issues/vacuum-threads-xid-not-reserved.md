---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 599
summary: vacuum_threads crashes_beside_concurrent_vacuum_recover_clean panics at seed 13 with Xid(0) is not a reserved id
closed: 2026-10-26
tags:
  - sim
  - engine
---

# Bug: vacuum crash recovery panics on an unreserved xid

## Report

`crates/adtb-sim/tests/vacuum_threads.rs` `crashes_beside_concurrent_vacuum_recover_clean` panics at seed 13 with `Xid(0) is not a reserved id`, in the assert at `crates/adtb/src/mvcc/clog.rs:394` inside `status`. The panicking thread is "other 1". Only the deep profile reaches seed 13; the default profile's sweep does not.

```
cargo nextest run --profile deep -p adtb-sim --test vacuum_threads
```

Reproduced on workspace rqxztvot with `SEEDS = 13..14`.

## 2026-10-26: done

Cause: a reader collected index entries, then walked each entry's TID. Between the two, vacuum removed the entry of an aborted insert, freed the emptied heap page, and the large-value store took the page. The store's pages are slotted heap pages, so `walk` read a chunk as a version header and handed its zero `xmin` to the commit log. The crash was incidental: it only shifted the schedule. `heap::walk` now takes the heap the start TID is in and ends the chain at a version on a page of the wrong owner: the first must be in that heap, the successor of a moved version in another heap and flagged retagged in, any other successor in its predecessor's heap. `exec::Store::fetch` and `write::apply::hot_run` pass their heap; `fetch` had the same exposure. The walk contract in `docs/design/mvcc.md` says so. Tests: `vacuum::a_walk_from_an_entry_read_before_vacuum_ends_on_a_page_the_large_store_took` and `vacuum_threads::crash_beside_vacuum_at_seed_13_recovers_clean`; the deep profile passes `vacuum`, `vacuum_threads`, `heap` and `heap_threads`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the ebug track.

---
created-at: 2026-11-09
status: resolved
kind: feature
priority: 380
summary: "the damage search injects one fault per case, so holes that need two faults (a lost write and a log gap, an unreadable list page and a crash mid-fold) are found only by review; add a tier of fault pairs judged against 'no damage record and no durable mark is ever lost'"
closed: 2026-11-10
tags:
  - testing
---

# Feature: fault pairs in the damage search

## Request

structure-damage-policy steps 4b and 5 each removed a mechanism that also covered an unstated case, and review found the holes by reasoning through sequences of two faults: records the list could not fold recycled with their log, a replayed free lost with the list page unreadable at open, a fold's bound cleared before its change was logged, a marks page loaded unverified written back over a lost mark. Each fix is pinned by one test for its scenario. The damage search (crates/adtb-sim/tests/storage/damage_search) injects one fault per case and cannot reach any of them.

## Proposal

A tier of the damage search that injects pairs: a lost or recycled write of one page with a log gap, an unreadable damage list or marks page, or a crash at each point of a checkpoint's fold. Judge two properties over every case, across closes, crashes and reopens: every damage record logged reaches the damage list or a durable mark, and every durable mark survives until released. The tier runs in a deep profile with a small default count. Schedule before structure-damage-policy closes, after its step 11.

## 2026-11-10: fixed

Landed a741ceaa, full check exit 0. Four pairs tests: a lost or recycled first write with a second fault from lost, recycled, torn, unreadable once or for good, a lost or unreadable log write, an unreadable damage list or marks page, and a crash at each point of a checkpoint's fold (log, force, header, list). Two properties are judged from the file by the search's own byte reader (`damage_search/ledger.rs`), at every process end: every damage record the log held reaches the damage list, the log past the redo LSN, a later free or image, or a durable database mark while the list page holds no list (§5.1, §6, §8); every durable mark stays until released and refuses the client's reads (§8). Two scenario tests show each judge reports (the step 4b recycled record, the step 5 marks page written back). Each second fault's floor counts cases whose planned fault fired, reads included; extra sequences that reach a floor are printed. Every allowance under a lost log write is tied to a sentence of storage.md §3, §6, §7 or §8. Found: verify-misses-stale-page-before-log-gap (9 `KNOWN` entries); `KNOWN` 10 to 19. Accepted: a clean close whose header names a checkpoint in a lost batch fails the next open as `Corrupt`. Unmeasured: an ack's place in the run log is read when the write returns, so a log write another task submits in between counts before it.

## 2026-11-10: resolved

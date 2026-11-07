---
created-at: 2026-11-06
status: resolved
kind: bug
priority: 330
summary: a real-file open_damage test found a page Stale with found Lsn(0) once under full-suite load; it passes alone
closed: 2026-11-06
tags:
  - class-not-atomic
---

# Bug: a real-file verify finds a blank page stale under load

## Report

In the migration-identity-sweep change 2 `check --changed` (2026-11-06, a tree without damage-log change 2), `open_damage::dropping_a_transaction_on_a_damaged_commit_log_of_a_real_file_does_not_panic` failed at open_damage.rs:604: verify returned `StalePage` for "page 13: Stale { expected: Lsn(5617569), found: Lsn(0) }" besides the expected note "the database: quarantined whole, so the findings of this run are not kept". Run alone it passes. The change touched only migration conversion.

A page reading blank with an expected LSN on a real file is either a write the test's own steps never made durable before the read (a test or harness race) or a page the engine wrote and lost: a healthy page refused, or a real lost write. Unknown which.

First step: run the test repeatedly under load (the suite's concurrency) and capture page 13's history: who allocated and wrote it, and whether the data force covered the write before verify read it.

## 2026-11-06: cause

Reproduced without outside load at about 0.9% (5 of 572 runs). Page 13 is the commit-log status page the test damages. `Pager::damage` evicts the frame and releases its latch before it inserts the page into `meta.damaged`; in that gap another thread's `Pager::resident` takes the latch, finds the page neither resident nor damaged, reads the blank file copy and refuses it as Stale with found Lsn 0, and its own `damage` call then overwrites the Checksum record (the last record wins). A false finding of the wrong kind; no acknowledged write lost. With the file quarantined whole the same gap could hand the reader the older file copy as data; production callers that refuse an in-memory page (clog.rs, heap/dir.rs) meet the same gap. Class: class-not-atomic, the refusal published after the latch that ends residency is released. Fix: publish the refusal under the exclusive latch before residency ends.

## 2026-11-06: the worse half is real

The fix track's simulator search, on main: with the file quarantined whole (the read-in gate off), a reader racing the refusal was handed the file's older copy as data (seed 1: byte 0xa1 after three reads of the in-memory 0xb2). With the gate armed, seed 13 recorded Stale over the real Checksum.

## 2026-11-06: fixed

`fix: publish a page refusal before ending its residency, under the latch`. Rule: a page's refusal is published (in `damaged` and the damage record) before its residency ends, under the same exclusive latch. `Pager::damage` latches, calls `publish_refusal(&latch, ..)` (damage_log.rs), then evicts; `publish_refusal` borrows the latch and `evict` consumes it, so the compiler orders them. `resident()` and verify's `judge_page` (two latched judgements, a bitmap look before each, the second keeping its latch) refuse under the latch that judged, through `refuse_latched`; `publish_refusal` logs no second record for a page already refused with the same damage. The other refusal sites judge resident or staged bytes or no page, so a reader in their gap gets the bytes it had. adtb-sys gains a simulator switch point after `Exclusive::evict`, without which the race was unreachable. Tests (damage_race.rs, 5): searches over 64 seeds of a refusal racing a reader (gate armed: refused as Checksum, one record; quarantined whole: never the file copy), verify racing a reader (one record), two readers racing a read-in (one record), each with a reach floor on reads begun inside the refusal window; and verify of a file quarantined whole refusing a lost write as Stale. On the parent: seed 13 recorded Stale over Checksum, seed 1 served the older file copy, and about 18% of seeds logged two records. The real-file test passed 300 of 300 stress runs, against 4 failures in 300 before. Review and delta review: a weak reach floor, a third judgement that broke a scrub test, and a test that pinned the stale copy, all fixed. `check --changed` exit 1 only in house-check, rerun clean; full `check` on main after landing.

## 2026-11-06: resolved

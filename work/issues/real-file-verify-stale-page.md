---
created-at: 2026-11-06
status: open
kind: bug
priority: 330
summary: a real-file open_damage test found a page Stale with found Lsn(0) once under full-suite load; it passes alone
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

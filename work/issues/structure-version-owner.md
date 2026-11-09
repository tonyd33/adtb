---
created-at: 2026-11-07
status: open
kind: feature
priority: 13
summary: "visibility, deadness, chains and version identity are judged from raw xmin and xmax at about 60 sites outside mvcc, with two deadness tables and two read-before checks"
---

# Feature: Row-version rules owned by mvcc

## Request

## Proposal

## Open questions

## 2026-11-07

Audit: work/briefs/audit-structure-r1-data.md (local). Also: 22 `storage::owner(` checks in heap at four strengths; `Heap::raw` asserts the owner before the version check (heap/handle.rs:318), so a reused page may panic where `Changed` is due; 5 copied heap commit sequences and 26 stage commits with no checked latch order; the catalog's roots heap as 16 hand-numbered constructors.

Exit: `.xmin`/`.xmax` outside mvcc at 0 behind an mvcc `Reader` and fate operation; one page-owner check in heap; heap writes through one staged operation that keeps the latch order.

## 2026-11-08, design and rulings

Design: work/briefs/design-version-owner-r1-report.md (local), eleven steps; this program is steps 1 to 10. Steps 1 to 6 change no behaviour: (1) `mvcc::Writers` replaces the header's raw `xmin`/`xmax`; (2) a `Reader` returning `Seen { visible, before }` held by `AtSnapshot`, `AtMarker` and the raw `sees` callers; (3) `Horizon`, `Fate` and one `fate` in mvcc, called by vacuum and history; (4) one `chain::roots`, `Writers::precedes`, a fallible `walk` that returns the version; (5) one `HeapWrite` that takes latches in rank order and owns the all-visible flag, VM bit, directory category, large-value retain and release, and settle; (6) `HeapPage`/`Listed` views over the guard's owner, `From<Tid>` for `WriteTarget` test-only. Steps 7 to 10 change behaviour, each with its pinning case: (7) `raw` answers `Changed` instead of asserting the owner; (8) an unreserved xid is refused, not a panic; (9) `version::link` requires `precedes`; (10) `retag` and catalog inserts prune on access.

Lead rulings: (Q1) heap imports mvcc's value types `Writers` and `Fate`, never `Transactions`; mvcc imports nothing of heap. (Q2) every heap insert path prunes on access (step 10); mvcc.md §10.2 is not narrowed. (Q3) an unreserved xid in a header is damage of the page holding it, refused through the damage the read path already reports for a structural fault; a typed shape-fault damage variant is a separate storage ticket, not this program. (Q4) a version moved to history is identified by its key and its writers, stated in mvcc.md §10.3; `Writers: Eq` implements it. (Q5) `version::link` takes the writers check (step 9). (Q6) the roots heap's hand-numbered constructors and `naming_sequences` leave this program for structure-catalog-roots. (Q7) the scoped-snapshot check belongs to structure-transactions; the `Reader` leaves room for its `Scope`.

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

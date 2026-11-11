---
created-at: 2026-11-11
status: open
kind: bug
priority: 490
summary: "the lock witnesses of rebuild, show_vacuum, set_retention, verify and admit_commit are unused, so a VacuumHold or Ordered taken from another database type-checks and proves nothing"
tags:
  - quality
  - engine
  - class-none
---

# A lock witness from another database is accepted

## Report

The witness parameters added for positive lock preconditions are unused: `rebuild` (crates/adtb/src/write/reindex.rs:467), `show_vacuum` (write/vacuums.rs:140), `set_retention` (write/past.rs:329), verify (write/verify.rs:284) and `admit_commit` (write/quarantine.rs:400) accept a `VacuumHold` or `Ordered` without checking it came from `self`. A witness taken from another `Database` (or another serializer) type-checks, so "this database's lock is held" has no enforcer. The same holds for `BundleWriter` from another `StoredLock`. Recorded as a residual of the transactions restructure, with no ticket.

Direction: tie each witness to its owner, by a lifetime borrowed from the database or by a check under `debug_assertions` or `latch-checks` that the witness names `self`.

Defect class: none.

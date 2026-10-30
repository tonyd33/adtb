---
created-at: 2026-10-30
status: open
kind: bug
priority: 439
summary: catalog transactions commit with no gate and outside the serialization lock; past_from_heaps grant and cancel untested; the vacuum hold test runs one seed that kills it
---

# Quarantine gate follow ups

## Report

Found in review of quarantine-gate. `mvcc.md` and `storage.md` section 8 say a commit checks marks; catalog transactions do neither. By reading only; no failing history built.

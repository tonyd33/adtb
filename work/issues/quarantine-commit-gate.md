---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 435
summary: "a transaction with writes applied before a mark can still commit; the gate checks statements, not the commit"
closed: 2026-10-30
---

# Quarantine commit gate

## Report

Transaction writes `identities.Google`, a reader then marks an index covering it, and the transaction commits. Ruling: commit checks its write set against the marks and fails with `Quarantined`.

## 2026-10-30

Landed on main. Check passed, 3130 tests.

## 2026-10-30: resolved

---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 432
summary: "a read of the past through a damaged history index fails Quarantined through fetch_past, while 11.2 says a quarantined index never fails a read"
closed: 2026-10-30
---

# Quarantine history index read

## Report

Catalog 11.2 says a quarantined index never fails a read. A read of the past through a damaged history key index reaches `fetch_past` in `exec/store.rs` and fails. Acceptance: such a read runs, without that index, and returns the same rows.

## 2026-10-30

Landed on main. Check passed, 3130 tests.

## 2026-10-30: resolved

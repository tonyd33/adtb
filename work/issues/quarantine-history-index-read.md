---
created-at: 2026-10-30
status: open
kind: bug
priority: 432
summary: "a read of the past through a damaged history index fails Quarantined through fetch_past, while 11.2 says a quarantined index never fails a read"
---

# Quarantine history index read

## Report

Catalog 11.2 says a quarantined index never fails a read. A read of the past through a damaged history key index reaches `fetch_past` in `exec/store.rs` and fails. Acceptance: such a read runs, without that index, and returns the same rows.

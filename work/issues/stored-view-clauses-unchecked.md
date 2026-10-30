---
created-at: 2026-10-29
status: open
kind: bug
priority: 535
summary: key and index clauses on a stored view are not checked against its rows
---

# Stored view clauses unchecked

## Report

Found by the view-panic track. `store v key nosuch` on `Set { n: Text }`, and `key name` or `index (name)` on `Set Text`, all apply with no refusal. derived.md 2.5.

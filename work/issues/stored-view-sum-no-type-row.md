---
created-at: 2026-10-29
status: open
kind: bug
priority: 540
summary: "a stored view's catalog::relations.sum names no catalog::types row"
---

# Stored view sum no type row

## Report

Found by the view-panic track. Holds for record views too, so it predates the fix of store-set-text-view-panic.

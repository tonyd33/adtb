---
created-at: 2026-10-30
status: open
kind: bug
priority: 560
summary: "dead versions of replaced findings accumulate in the roots heap, which no vacuum reaches"
---

# Verify findings dead versions

## Report

Found by the verify-online track. Each verify run that replaces findings leaves dead versions in the heap of kept rows.

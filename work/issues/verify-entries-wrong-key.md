---
created-at: 2026-10-30
status: open
kind: bug
priority: 480
summary: the library entries check still accepts a live slot of the wrong key
---

# Verify entries wrong key

## Report

Found by the verify-online track; receipt-verify-online.md. An index entry naming a live slot whose row has another key passes the check.

---
created-at: 2026-10-29
status: open
kind: bug
priority: 770
summary: Two updates of one row in a block are reported as an insert key collision
---

# Two updates of one row in a block are reported as an insert key collision

## Report

Found by the manual reader check on 2026-10-29. At
docs/manual/concepts/serializable-transactions.md:88, two updates of one row in one block fail
with "`bank::accounts` already holds a row with key ...", which names an insert collision. Fix:
name the row updated twice in the block; update the manual page.

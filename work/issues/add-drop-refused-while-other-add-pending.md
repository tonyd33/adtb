---
created-at: 2026-11-16
status: open
kind: bug
priority: 450
summary: a migration that adds and then drops a relation is refused whenever another relation it adds later is still pending
tags:
  - lang
---

# Add drop refused while other add pending

## Report

Found by the review of committed-schema-type part 4a-1; refused the same on main 9e4804be and on the part's tree. Engine script (shop and customers fixtures):

```
migrate
  module shop
  add type Stamp = { id: Key stamps, label: Text }
  add relation stamps : Stamp key id generate sequence
  drop relation stamps
  drop type Stamp
  add type Mark = { id: Key marks, label: Text }
  add relation marks : Mark key id generate sequence
```

Expected: the migration applies, and `insert marks { label: "x" }` then reads back `"x"`. Actual: refused with "a later operator of the migration rewrites the declaration this operand was written into, so its names do not resolve". While `marks` is pending, `names_built` treats the schema as unsettled, so the first operator's marks are deferred to the late pass, and the drop rewrites the declaration they were written into.

Cause not yet established beyond that path; the defect class will be tagged when it is.

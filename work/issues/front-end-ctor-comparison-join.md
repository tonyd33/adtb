---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 96
summary: comparing values of two constructors of one sum is refused instead of checking at their join
branch: lang
tags:
  - lang
---

# Comparing values of two constructors of one sum is refused

## Report

Found by the reference interpreter.

```adt
Sent { at: 100 } < Paid { at: 0, ref: "a" }
```

Actual: `expected Stage.Sent, found Stage.Paid`.

Expected: the operands meet at their join, `Stage`, and the comparison
checks.

---
created-at: 2026-11-17
status: open
kind: bug
priority: 400
summary: "a deref in an index expression is refused by the engine and accepted by the reference, which reports Violated"
tags:
  - lang
  - class-none
---

# Index expression deref diverges

## Report

Found by the review of constraint-deref on 2026-11-17, predating that change. A deref in an index expression is refused by the engine at migrate ("the row of a key in a scalar expression") and accepted by the reference, which then reports Violated for a dangling key. The checker admits it and makes its deref total, though an index expression reads the row being written before checks run.

Expected: the checker refuses a deref in an index expression, naming it, so the reference and the engine agree; or the spec admits it and the engine supports it. The rule belongs in the checker, once.

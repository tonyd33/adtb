---
title: "Whether a constraint or check body may be partial"
pick: one
options:
  accepted:
    title: "Partial bodies are accepted; a fault while checking fails the statement"
  total-only:
    title: "A partial constraint or check body is refused at declaration"
---
Raised by the totality analysis. constraints.md said what a fault
while checking does, but not whether a partial body is admitted.

```adt fragment
check ratio on items (total / qty < 100)   -- faults when qty == 0
```

## spec/constraint-faults/accepted

A constraint or check body may be partial. A fault while checking it
fails the statement with that fault, ahead of any violation.

## spec/constraint-faults/total-only

Only total bodies are admitted. Every division and conversion has to
be written through a `checked_` form or `try`, and a body calling a
partial schema function is refused.

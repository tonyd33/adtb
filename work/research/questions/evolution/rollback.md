---
title: How a migration is undone
pick: one
raised-by: [evolution/smo]
options:
  revert:
    title: revert appends the inverse operators as a new migration
  none:
    title: No rollback; write a new migration by hand
---
Operators are closed under inverses, so an undo is computable.

## evolution/revert

`0013 revert 0011` appends the inverses of 0011's operators; the log
only grows, and versions are never reused.

## evolution/none

Hand-written forward fixes only.

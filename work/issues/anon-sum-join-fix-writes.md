---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 185
summary: "anonymous sums joined by fix binders, upsert rows and update match arms are not renumbered into the joined sum"
branch: adtb-lang2
tags:
  - lang
---

# Bug: Anonymous sum joins in fix binders and writes

## Report

Ad hoc joins of anonymous sums renumber each operand's constructors to
their index in the joined sum, in name order, for `if` branches, `match`
arms, `union` operands, set literal elements and comparison operands.
Three joins do not: the binder type of a `fix` joined across `union`
operands, upsert row joins, and the arms of an update `match`. These may
only meet declared row types, where no renumbering is needed. Either
show that with a test per join, or renumber there too.

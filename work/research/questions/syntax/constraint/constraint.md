---
title: "How constraints are declared and named"
pick: one
options:
  named:
    title: "Every constraint and check is named"
  anonymous:
    title: "Unnamed checks allowed"
---
Constraints dropped or redefined by operators need names.

## syntax/constraint/named

`constraint balanced = from e in ...`; row checks `check positive on lines (amount > 0)`; `drop constraint balanced`.

## syntax/constraint/anonymous

`check lines (amount > 0)`, dropped by restating the expression.

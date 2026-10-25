---
title: "What an upsert binds"
pick: one
options:
  change-set:
    title: "The row with old's key among the else branch's effects, or old; defined from the change set"
  syntax:
    title: "Read off the syntax of the else branch"
---
Raised by the implementability review of docs/spec/. semantics §10.3
found the binding of an upsert through a table over the syntax of the
else branch, so two branches with one change set could bind different
rows, and the binding's type was the target row type although the
branch may retag.

## spec/writes/upsert-binding/change-set

The binding of an upsert is the row with `old`'s key among the else
branch's effects, or `old` when the branch leaves it unchanged; defined
from the change set, not the syntax of the branch. Its type is the join
of the target row type and the types of the branch's new values.

## spec/writes/upsert-binding/syntax

A syntax-directed table finds the new value. A nested `let`, a write
sequence or a `match` changes what is bound without changing the
change set.

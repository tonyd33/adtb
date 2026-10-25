---
title: "When an upsert's else branch can delete the row"
pick: one
options:
  through-actions:
    title: "Any delete of old the branch can cause, delete actions included"
  direct-only:
    title: "Only a delete the branch writes of old itself"
---
Raised by the writes checker. writes §6.2 refuses a `let` on a
single-row upsert whose else branch can delete the row, and did not say
whether a delete reached through a delete action counts.

```adt fragment
-- given u: Key users, k: Key logins
transact
  let s = insert logins.Revoked { id: k, user: u }
          unless conflict on key
          else delete x in users where key x == u   -- cascades to old
  return key s
```

## spec/writes/upsert-delete/through-actions

The branch can delete the row when any arm deletes `old`, or deletes
from a relation whose `cascade` actions, followed transitively, reach
`old`'s relation. Such a `let` is a type error.

## spec/writes/upsert-delete/direct-only

Only an arm that deletes `old` counts. A cascade from another delete
then removes the row the `let` is bound to, so the binding names a row
the statement deletes.

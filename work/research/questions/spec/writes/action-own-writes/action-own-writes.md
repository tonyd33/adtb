---
title: "Whether delete actions reach rows the statement itself writes"
pick: one
options:
  statement-wins:
    title: "Actions reach only pre-state rows the statement neither replaces nor deletes; a replaced row is judged by its new value"
  pre-state-anyway:
    title: "Actions apply to the pre-state row even when the statement replaces it"
---
Raised while applying the ruling that delete actions reach only
pre-state rows. A statement may update a referrer and delete its
target together, and the spec let the action derive a second new
value for the row the statement already replaced.

## spec/writes/action-own-writes/statement-wins

Delete actions reach only pre-state rows that the statement itself
neither replaces nor deletes. A row the statement replaces is judged
by its new value: if it references a gone key, its reference
constraint is violated, as for an inserted row. Repointing a reference
and deleting its old target in one statement succeeds:

```adt
transact
  update l in logins.Active where l.identity == old set Active { l with identity: new }
  delete i in identities where key i == old
```

## spec/writes/action-own-writes/pre-state-anyway

Actions apply to the pre-state row whatever the statement does to it.
The example above then gives the login two new values and fails with
`KeyViolation`.

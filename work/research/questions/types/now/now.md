---
title: What the current time is inside a pure query
pick: one
raised-by: [model/time]
options:
  snapshot-time:
    title: A clock reading fixed at transaction start
  param-only:
    title: No clock in queries; times come from parameters or generated fields
  commit-time:
    title: A clock reading taken at commit, usable only as a written field value
---
Queries are pure; a write is a change-set value (`write/surface/txdata`).
Under SSI the serial order is not the start or commit order
(`engine/ssi`), so any clock reading may disagree with it.

## types/now/snapshot-time

Postgres `now()`: the same value for the whole transaction.

## types/now/param-only

```
query due (at: Instant) -> Set Task = from t in tasks where t.due < at select t
```

A registered query is a function of its parameters and the snapshot.

## types/now/commit-time

Datomic `:db/txInstant` on the transaction entity
(https://docs.datomic.com/schema/schema-reference.html, unverified).
A field `created: Instant = generate commit_time` is filled when the
change set commits, and is monotone in commit order.

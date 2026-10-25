---
title: A multi-step online change cannot roll back with its transaction
pick: any
raised-by: [[engine/transactional-ddl, engine/online-states]]
options:
  staged-job:
    title: The transaction commits the first state, a job runs the rest
  live:
    title: Live with schema changes outside user transactions
    excludes: [engine/ddl-online-txn/staged-job]
---
Each intermediate state must be committed and seen by every running
transaction before the next begins, so the change spans several
commits.

## engine/ddl-online-txn/staged-job

CockroachDB: the user transaction commits the first state; later states
run as a background job, and failure runs the inverse operators
(https://www.cockroachlabs.com/docs/stable/online-schema-changes,
unverified).

```
0011 add constructor Identity.Apple { sub: Text }
-- job: declared, then writable, or rolled back by remove constructor Identity.Apple
```

## engine/ddl-online-txn/live

Online changes run only outside transactions.

---
title: Which non-default levels a transaction may choose
pick: some
raised-by: [engine/serializable-default, engine/snapshot-default]
options:
  opt-snapshot:
    title: Snapshot isolation
  opt-read-committed:
    title: Read committed, a fresh snapshot per statement
    links:
      - {rel: prior-art, to: prior/engine-distributed-sql}
---
Weaker levels trade anomalies for fewer aborts. Each one offered must
say what happens to commit-time constraint checks and to a retag racing
an update.

## engine/opt-snapshot

```
transaction isolation snapshot { ... }
```

Postgres REPEATABLE READ: one snapshot, concurrent update of a written
row aborts.

## engine/opt-read-committed

```
transaction isolation read_committed { ... }
```

Each statement sees the latest committed state; an update blocked on a
concurrently updated row re-evaluates on the newest version (Postgres
EvalPlanQual, https://www.postgresql.org/docs/current/transaction-iso.html).

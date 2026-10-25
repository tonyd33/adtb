---
title: A commit-time failure is reported far from the write that caused it
pick: any
raised-by: [constraint/timing/commit]
options:
  blame:
    title: The error names the statement and row that broke the constraint
  check-now:
    title: A statement that checks pending constraints mid-transaction
    links:
      - {rel: prior-art, to: prior/write-pg-constraints}
  live:
    title: Live with errors at commit only
    excludes: [write/late-error/blame, write/late-error/check-now]
---
Every key and FK violation surfaces at `commit`, after the program has
read state that violates it. The Third Manifesto's objection: a user
sees a state in which a constraint fails.

## write/late-error/blame

Each pending violation records the write that introduced it.

## write/late-error/check-now

```
check constraints;
```

Postgres `SET CONSTRAINTS ALL IMMEDIATE` checks pending deferred
constraints at that point.

## write/late-error/live

The transaction fails at commit with the violated constraint's name.

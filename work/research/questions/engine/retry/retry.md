---
title: Serializable transactions abort and must be retried
pick: any
raised-by: [engine/serializable-only, engine/serializable-default, engine/strict-serializable]
options:
  retry-block:
    title: A transaction block is a retryable unit the client library reruns
    links:
      - {rel: prior-art, to: prior/engine-fdb}
  safe-snapshot:
    title: Declared read-only transactions take a safe snapshot and never abort
    links:
      - {rel: prior-art, to: prior/engine-pg-ssi}
  live:
    title: Live with applications handling serialization failures
    excludes: [engine/retry/retry-block, engine/retry/safe-snapshot]
---
Every serializable mechanism except deterministic ordering aborts some
transactions. Long analytic reads under SSI or OCC are the usual victim.

## engine/retry/retry-block

The block has no effects outside the database, so rerunning it is
safe; the library retries on serialization failure, as FoundationDB
`db.run` and CockroachDB client retry loops do.

```
transact retry 5
  let i = insert identities.Google { user: u, gid: "x" }
  return key i
```

## engine/retry/safe-snapshot

Postgres `SERIALIZABLE READ ONLY DEFERRABLE`: wait until a snapshot has
no concurrent read-write transactions that could make it unsafe, then
run without predicate tracking.

```
transact read_only deferrable
  return count(i for i in identities)
```

## engine/retry/live

Errors surface; callers loop.

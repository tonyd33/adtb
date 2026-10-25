---
title: Deterministic ordering needs read and write sets in advance
pick: any
raised-by: [engine/deterministic]
options:
  registered-txn:
    title: Only registered transactions whose sets the compiler derives
    links:
      - {rel: gains, to: evolution/breaks/invisible/registered}
  reconnaissance:
    title: Dependent transactions run a reconnaissance read first
    links:
      - {rel: prior-art, to: prior/engine-hekaton-calvin}
  live:
    title: Live without interactive transactions
    excludes: [engine/deterministic-sets/registered-txn, engine/deterministic-sets/reconnaissance]
---
A transaction that reads `users` to decide which `identities` row to write
has a write set fixed only by its reads.

## engine/deterministic-sets/registered-txn

Transactions are stored procedures in the query language; the compiler
derives the constructor relations and key ranges touched.

```
txn add_google (u: Key users, g: Text) -> Key identities =
  transact
    let i = insert identities.Google { user: u, gid: g }
    return key i
```

## engine/deterministic-sets/reconnaissance

Calvin's OLLP: read to estimate the sets, then submit; abort and redo
if the estimate was wrong.

## engine/deterministic-sets/live

Clients submit whole transactions; no read-then-decide round trips.

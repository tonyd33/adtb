---
title: What state a session holds beyond authentication and bundle
pick: one
raised-by: [surface/wire/binary]
options:
  stateless:
    title: None; every call is self-contained, a transaction is named by an id
  transaction:
    title: Only the open transaction, if any
  settings:
    title: Session settings and variables as well
    assessment: {grade: costly, why: state a pooler must track or forbid; PgBouncer forbids SET in transaction mode}
    links:
      - {rel: prior-art, to: prior/net-pgbouncer}
---
Registered queries remove prepared statements from the session; what
remains decides whether a pooler can share server connections.

## net/session/stateless

```
-> Begin  <- Txn 0x91
-> Call { txn: 0x91, query: 3, args }   -- may arrive on any connection
```

Flight SQL transaction handles; a connection is a pipe.

## net/session/transaction

Postgres in transaction-pooling mode, minus the broken features.

## net/session/settings

`set timezone`, `set statement_timeout`: Postgres session GUCs.

---
title: Who reruns a single-call transaction after a serialization failure
pick: one
raised-by: [write/transact/registered-txn]
options:
  server:
    title: The server reruns it before replying, up to a limit
    links:
      - {rel: prior-art, to: prior/net-cockroach}
      - {rel: prior-art, to: prior/net-convex-sync}
  client:
    title: The error goes to the client library, which resends
    links:
      - {rel: prior-art, to: prior/net-fdb-client}
---
A registered transaction has no effects outside the database
(`engine/retry/retry-block`), so whoever holds it can rerun it.

## net/retry/server

```
-> Call sign_up (n, g)
   (aborted by SSI, rerun at a new snapshot, committed)
<- End { key: 0x51, attempts: 2 }
```

CockroachDB retries implicit transactions while no result has reached
the client; Convex reruns mutations. Nothing has reached the client
until commit, so every attempt qualifies.

## net/retry/client

The error crosses the network each time.

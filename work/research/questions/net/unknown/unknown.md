---
title: A connection lost during commit leaves the outcome unknown
pick: any
raised-by: [engine/retry/retry-block]
options:
  idempotency:
    title: Each write call carries a client request id the engine deduplicates
    links:
      - {rel: prior-art, to: prior/net-tigerbeetle-client}
  status:
    title: Commit returns a transaction id first; the client asks its outcome after reconnecting
  live:
    title: Live with an unknown result surfaced to the caller
    excludes: [net/unknown/idempotency, net/unknown/status]
    links:
      - {rel: prior-art, to: prior/net-fdb-client}
---
A retry block rerun after a lost commit acknowledgement may apply
twice. FoundationDB's `commit_unknown_result`: committed, or never
will.

## net/unknown/idempotency

```
-> Call { query: sign_up, request: (client 0x3a…, 812), args }
<- End ...   -- a resend of (0x3a…, 812) gets the stored reply
```

TigerBeetle sessions with request numbers. The dedup table is a
catalog relation with a retention window.

## net/unknown/status

```
-> Commit   <- Committing { txn: 0x91 }   ... connection lost ...
-> Status 0x91   <- Committed { at: 0/1A2B }
```

## net/unknown/live

The application keeps its own idempotency keys.

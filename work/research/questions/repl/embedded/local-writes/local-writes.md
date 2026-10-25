---
title: An embedded replica writes while the primary is far or offline
pick: any
raised-by: [repl/embedded/embedded-replica]
options:
  forward:
    title: Every read-write transaction runs on the primary
  validate-remote:
    title: The replica runs the transaction and ships its change set with read version and read set; the primary validates
    links:
      - {rel: prior-art, to: prior/engine-fdb}
      - {rel: gains, to: mvcc/long-reader/ctor-scoped}
  live:
    title: Live with writes failing while the primary is unreachable
    excludes: [repl/embedded/local-writes/forward, repl/embedded/local-writes/validate-remote]
---
libSQL `offline: true` writes locally but still sends write
transactions containing reads to the primary.

## repl/embedded/local-writes/forward

One round trip per transaction; serializable by the primary's SSI.

## repl/embedded/local-writes/validate-remote

FoundationDB's resolvers check a transaction's read set against writes
committed after its read version. A registered transaction names the
constructor relations and key ranges it reads, so the replica can
ship them. Backward validation is stricter than SSI (it aborts every
read that was overwritten), so it is sound beside SSI on the primary
(unverified how the two conflict detectors compose).

```
replica tx @0/1A2B reads { users[3], identities.Google[user == 3] }
              writes { identities.Google + { ... } }
```

## repl/embedded/local-writes/live

Offline means read-only.

---
title: Which client and server releases interoperate
pick: one
raised-by: [surface/wire/binary]
options:
  negotiate:
    title: The client states a protocol version; the server answers with the highest it supports
    links:
      - {rel: prior-art, to: prior/ops-pg-limits}
  window:
    title: A server accepts clients from a stated window of past releases; never newer ones
    links:
      - {rel: prior-art, to: prior/ops-tigerbeetle}
  lockstep:
    title: Client and server must be the same release
---
Embedding has no skew: the library is the engine. The server protocol
carries per-query type descriptors and stable constructor tags
(proposed `surface/sum-encoding/stable-tag`), so data encoding does not
change with the schema; the protocol still changes with releases.

## ops/protocol/negotiate

Postgres: major mismatch refused, minor answered by
`NegotiateProtocolVersion`; 3.2 in PG 18 after 3.0 since 7.4.

```
-> Startup { protocol: 1.4, features: [trace, deadline] }
<- Negotiate { protocol: 1.3, features: [trace] }
```

## ops/protocol/window

TigerBeetle: a client may not be newer than the cluster; older clients
within a release's stated range work.

## ops/protocol/lockstep

Driver and server upgraded together; simplest, hostile to rolling
upgrades of either side.

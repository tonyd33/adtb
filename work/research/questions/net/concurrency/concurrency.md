---
title: How many calls one connection carries at once
pick: one
raised-by: [surface/wire/binary]
options:
  serial:
    title: One call at a time; the client waits for each result
    assessment: {grade: costly, why: a round trip per call and a connection per concurrent caller}
    links:
      - {rel: prior-art, to: prior/net-mysql}
  pipelined:
    title: Calls sent without waiting, executed and answered in order
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
  multiplexed:
    title: Independent concurrent calls and transactions, answered as each finishes
    links:
      - {rel: prior-art, to: prior/net-tds-mars}
      - {rel: prior-art, to: prior/net-quic}
---
A connection's concurrency sets how many connections a client needs,
which is the pooling problem.

## net/concurrency/serial

MySQL classic protocol; TigerBeetle sessions (one in flight, batched
by the client).

## net/concurrency/pipelined

```
-> Call a -> Call b -> Call c -> Sync
<- End a  <- End b  <- End c  <- Ready
```

Postgres pipeline mode: after an error the server skips to Sync, so a
pipeline is an implicit unit.

## net/concurrency/multiplexed

```
-> 1 Begin   -> 2 Call report (read_only)
-> 1 Call find_user   <- 2 Batch ...   <- 1 Batch ...
```

Each stream has its own transaction or none. MARS interleaves only at
yield points and scopes a transaction to its batch; here streams are
separate sessions sharing one socket and one authentication.

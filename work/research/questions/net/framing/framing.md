---
title: How frames relate requests to responses
pick: one
raised-by: [net/base/own]
options:
  sequential:
    title: Responses follow requests in order; no request ids
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
      - {rel: prior-art, to: prior/net-mysql}
  request-id:
    title: Each frame carries a stream id; responses of different streams interleave
    links:
      - {rel: prior-art, to: prior/net-mongo-wire}
      - {rel: prior-art, to: prior/net-tds-mars}
---
Decides whether one connection can carry concurrent calls, how
cancellation names its target, and where server pushes go.

## net/framing/sequential

```
-> Call a  -> Call b  -> Sync
<- Batch a ... End a  <- Batch b ... End b  <- Ready
```

Postgres extended protocol; pipelining is still possible.

## net/framing/request-id

```
[stream u32][type u8][len u32][body]
-> 1 Call login_key    -> 2 Call list_sessions
<- 2 Batch  <- 1 Batch  <- 2 End  <- 1 End
```

MongoDB `requestID`/`responseTo`; HTTP/2 and QUIC streams by
another name. Stream 0 carries pushes.

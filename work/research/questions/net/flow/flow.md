---
title: How a slow reader holds back a result stream
pick: one
raised-by: [surface/wire/binary]
options:
  pull:
    title: The client asks for each next batch
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
  credit:
    title: The client grants credit in bytes or batches per stream; the server sends up to it
    links:
      - {rel: prior-art, to: prior/net-quic}
  socket:
    title: The server writes until the socket blocks
---
Push pipelines (`exec/flow/push`) must stall somewhere when the
client is slow, and a stalled query holds its snapshot.

## net/flow/pull

Postgres Execute with a row limit, then PortalSuspended. A round trip
per batch.

## net/flow/credit

```
-> Call 3 { credit: 4 batches }  <- Batch <- Batch  -> Credit 3 +4
```

QUIC MAX_STREAM_DATA, HTTP/2 WINDOW_UPDATE. Works per stream under
multiplexing.

## net/flow/socket

Simple; under multiplexing one slow stream blocks the connection.

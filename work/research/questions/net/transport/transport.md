---
title: Which transports the server listens on
pick: some
raised-by: [surface/wire/binary]
options:
  tcp:
    title: TCP
  unix:
    title: Unix domain sockets for same-host clients
  quic:
    title: QUIC, one stream per call
    links:
      - {rel: prior-art, to: prior/net-quic}
  websocket:
    title: WebSocket for browser and edge clients
    links:
      - {rel: prior-art, to: prior/net-convex-sync}
      - {rel: prior-art, to: prior/net-spacetimedb-sync}
---
These stack; the framed protocol is the same over each byte stream.

## net/transport/tcp

```
adt://db.internal:7433/app
```

## net/transport/unix

```
adt:///run/adt/app.sock
```

Postgres local sockets; peer credentials name the OS user.

## net/transport/quic

Independent streams avoid head-of-line blocking between concurrent
calls; per-stream and per-connection flow control; 0-RTT has no replay
protection (RFC 9000 2.1, 4, 17.2.3), so only read-only calls may ride
it.

## net/transport/websocket

```
wss://db.example/app   -- same frames, one per WebSocket message
```

Convex and SpacetimeDB serve clients this way.

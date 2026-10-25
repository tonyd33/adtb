---
title: How a running call is cancelled
pick: one
raised-by: [surface/wire/binary]
options:
  in-band:
    title: A Cancel frame on the same connection names the stream
    needs: [net/framing/request-id]
  side-channel:
    title: A new connection presents a secret key from the handshake
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
  reset:
    title: Resetting the transport stream cancels its call
    needs: [net/transport/quic]
---
Cancellation ends a call; inside an interactive transaction it also
decides whether the transaction survives.

## net/cancel/in-band

```
-> 5 Call report   -> 0 Cancel { stream: 5 }   <- 5 Error Cancelled
```

The server reads the socket while executing, which multiplexing
already needs.

## net/cancel/side-channel

Postgres CancelRequest with PID and secret key, 256-bit in 3.2; the
cancel races with the next call on the connection.

## net/cancel/reset

QUIC RESET_STREAM / STOP_SENDING, HTTP/2 RST_STREAM.

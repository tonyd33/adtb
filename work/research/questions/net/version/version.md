---
title: How the protocol version is negotiated
pick: one
raised-by: [surface/wire/binary]
options:
  range:
    title: The client offers a min and max version; the server picks one or refuses
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
  hello:
    title: A first Hello names one version; an unknown one is refused with the supported list
    links:
      - {rel: prior-art, to: prior/net-resp3}
  alpn:
    title: The TLS ALPN id names the protocol version
    needs: [net/tls/direct]
---
Protocol version, not schema version. Postgres 3.1 was never used
because a pooler mishandled it, so negotiation must survive
intermediaries.

## net/version/range

```
-> Hello { proto: 1.0..1.3, features: [push, arrow] }
<- HelloOk { proto: 1.2, features: [push] }
```

Postgres NegotiateProtocolVersion, `min_protocol_version` and
`max_protocol_version` in libpq 18.

## net/version/hello

RESP3 `HELLO 3`.

## net/version/alpn

`alpn: ["adt/2", "adt/1"]`; settled before any byte of the protocol.

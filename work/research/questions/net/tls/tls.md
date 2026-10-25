---
title: Whether and how TLS is required
pick: one
raised-by: [surface/wire/binary]
options:
  direct:
    title: TLS from the first byte on network transports with ALPN; plaintext only on Unix sockets
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
  upgrade:
    title: An in-band request upgrades the connection to TLS, which the server may require
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
  optional:
    title: TLS optional per listener
---
Postgres began with an upgrade request and added direct TLS in 17 to
save a round trip; the upgrade path needed a fix for buffer stuffing
(CVE-2021-23222).

## net/tls/direct

```
ClientHello { alpn: ["adt/1"] }  -> TLS 1.3 -> Hello { ... }
```

ALPN guards against protocol confusion, as Postgres requires for
`sslnegotiation=direct`. Channel binding for SCRAM is always
available.

## net/tls/upgrade

```
-> TlsRequest   <- 'S'   ... handshake ... -> Hello
```

## net/tls/optional

A listener flag; plaintext TCP allowed.

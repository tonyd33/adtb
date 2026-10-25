---
title: How a client authenticates
pick: some
raised-by: [surface/wire/binary]
links:
  - {rel: gains, to: security/assert}
options:
  scram:
    title: SCRAM-SHA-256 with channel binding
    links:
      - {rel: prior-art, to: prior/net-pg-auth}
  mtls:
    title: Client certificates mapped to principals
    needs: [net/tls/direct]
    links:
      - {rel: prior-art, to: prior/net-flight}
  token:
    title: Bearer tokens (OAuth or signed) checked by a validator
    links:
      - {rel: prior-art, to: prior/net-pg-auth}
  peer:
    title: OS peer credentials on Unix sockets
    needs: [net/transport/unix]
---
These stack per listener. They establish the connection's role;
which principal a call runs as and what it may touch is
`security/assert` and the `security` questions.

## net/authn/scram

```
-> Hello { auth: SCRAM-SHA-256-PLUS, client-first }
<- AuthContinue { server-first }  -> AuthFinal { client-final }  <- HelloOk
```

Postgres SASL; `tls-server-end-point` binding defeats a relaying
proxy, which matters if `net/pool/proxy` is taken.

## net/authn/mtls

Service-to-service; Flight lists mTLS.

## net/authn/token

Postgres 18 `oauth`; browser and serverless clients over WebSocket.

## net/authn/peer

Postgres `peer`.

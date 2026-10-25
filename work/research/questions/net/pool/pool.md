---
title: Where connections are pooled
pick: some
raised-by: [engine/embedded-server]
options:
  none-needed:
    title: Connections are cheap; no pooler between client and server
    needs: [net/server/async]
  client:
    title: Client libraries pool connections
  proxy:
    title: An external pooling proxy multiplexes many clients onto few server sessions
    links:
      - {rel: prior-art, to: prior/net-pgbouncer}
---
The PgBouncer problem is server sessions that are expensive and
stateful. Registration removes prepared statements; session state is
`net/session`.

## net/pool/none-needed

Serverless clients connect directly.

## net/pool/client

Every driver pools; multiplexing makes one connection per process
enough.

## net/pool/proxy

A proxy must speak the protocol, pass bundle binding through and
route cancels; Postgres 3.1 was skipped because of a pooler bug.

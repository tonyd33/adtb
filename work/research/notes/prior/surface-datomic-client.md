---
title: Datomic client and peer APIs
---
Queries are data (`[:find ?e :where ...]`), not strings; `db` returns an
immutable database value, `as-of` and `since` move it in time, `with`
applies a transaction speculatively. The peer library runs in process;
the client API talks to a server.
https://docs.datomic.com/client-api/datomic.client.api.html

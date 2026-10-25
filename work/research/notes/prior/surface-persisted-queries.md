---
title: GraphQL persisted queries
---
Relay: the compiler hashes each operation, emits a query map the server
stores, and the client sends only the id; the server can allowlist.
Apollo APQ: registration is lazy, the first hash miss answers
`PERSISTED_QUERY_NOT_FOUND` and the client resends the text, so every
query is seen only once it has run.
https://relay.dev/docs/guides/persisted-queries/
https://www.apollographql.com/docs/apollo-server/performance/apq

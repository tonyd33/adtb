# Apollo Server

kind: system
home: https://www.apollographql.com/docs/apollo-server

## Automatic persisted queries

- First hash-only request answers `PERSISTED_QUERY_NOT_FOUND`; the client retries with text and hash, which the server caches; every query string must be sent at least once. [V 2026-10-25] https://www.apollographql.com/docs/apollo-server/performance/apq

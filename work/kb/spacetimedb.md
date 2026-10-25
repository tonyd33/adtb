# SpacetimeDB

kind: system
home: https://spacetimedb.com

## Encoding

- BSATN encodes a SumValue as `bsatn(tag) ++ bsatn(variant_data)` with `tag: u8` an index into `SumType.variants`; a variant with no data has an empty payload. [V 2026-10-25] https://spacetimedb.com/docs/bsatn
- Client bindings are generated from the module schema by `spacetime generate`. [U]

## WebSocket protocol

- v1 ClientMessage: CallReducer, Subscribe, OneOffQuery, SubscribeSingle, SubscribeMulti, Unsubscribe, UnsubscribeMulti, CallProcedure. ServerMessage: InitialSubscription, TransactionUpdate, TransactionUpdateLight, IdentityToken, OneOffQueryResponse, SubscribeApplied, UnsubscribeApplied, SubscriptionError, SubscribeMultiApplied, UnsubscribeMultiApplied, ProcedureResult. Subscriptions are SQL query strings. [V 2026-10-25] https://raw.githubusercontent.com/clockworklabs/SpacetimeDB/master/crates/client-api-messages/src/websocket/v1.rs
- QueryUpdate is deletes plus inserts per transaction (initial subscription: inserts only); per-query updates may be Brotli or Gzip compressed. UpdateStatus: Committed, Failed (rolled back), OutOfEnergy. [V 2026-10-25] same file
- Protocol modules v1, v2, v3 exist. [V 2026-10-25] https://raw.githubusercontent.com/clockworklabs/SpacetimeDB/master/crates/client-api-messages/src/websocket.rs

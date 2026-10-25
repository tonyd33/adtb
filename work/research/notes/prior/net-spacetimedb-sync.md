---
title: SpacetimeDB WebSocket protocol
---
CallReducer invokes a transactional function by name with BSATN
arguments; Subscribe registers query strings. TransactionUpdate sends,
per subscribed query, the deleted and inserted rows of each committed
reducer, optionally Brotli or Gzip compressed.
https://raw.githubusercontent.com/clockworklabs/SpacetimeDB/master/crates/client-api-messages/src/websocket/v1.rs

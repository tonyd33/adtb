---
title: How the server protocol relates to the in-process API
pick: one
raised-by: [engine/embedded-server]
options:
  mirror:
    title: Each protocol message is an in-process API call serialized; the server is a client of the library
  separate:
    title: The protocol has its own operations beyond the library API
---
`engine/embedded-server` puts a server around an embedded core.

## net/parity/mirror

```rust
db.call(queries::login_key, id)          // in process
// -> Call { query: 3, args: [id] }       // over the wire, same semantics
```

Codegen emits one client trait with two implementations.

## net/parity/separate

Pooling, admission, feeds and pushes exist only on the server.

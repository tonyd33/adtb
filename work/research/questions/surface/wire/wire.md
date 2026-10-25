---
title: How clients reach the engine
pick: some
options:
  binary:
    title: A binary protocol with per-query type descriptors
    links:
      - {rel: prior-art, to: prior/surface-gel-migrations}
  http:
    title: Registered queries called over HTTP with a JSON body
    links:
      - {rel: prior-art, to: prior/surface-convex}
  embedded:
    title: An in-process library linking the engine
    links:
      - {rel: prior-art, to: prior/surface-datomic-client}
---
No SQL wire protocol to reuse. These stack over one engine.

## surface/wire/binary

```
-> Execute { query: #7f3c, bundle: web-2.3, args: [key] }
<- Descriptor { id: d41, Sum { 1 ByEmail { email: Text }, 2 ByGoogle { gid: Text } } }
<- Data [ tag 2, "g-123" ]
```

Gel's protocol sends input and output descriptors on Parse; a client
naming a stale descriptor id gets the new one.

## surface/wire/http

```
POST /q/login_key  {"u": "…"}   ->   {"ByGoogle": {"gid": "g-123"}}
```

Convex functions, GraphQL. Readable from any language without a
driver.

## surface/wire/embedded

```rust
let db = adtb::open("app.db")?;
for row in db.run(queries::login_key, u)? {
    match row { LoginKey::ByEmail { email } => ..., LoginKey::ByGoogle { gid } => ... }
}
```

SQLite, DuckDB, the Datomic peer. Results are host values with no
encoding step; registration happens at compile time of the host
program.

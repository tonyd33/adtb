---
title: How a client query is registered
pick: one
raised-by: [evolution/breaks/invisible/registered]
options:
  named:
    title: Named queries with signatures in a catalog, called by name
  hashed:
    title: Query text persisted by content hash
    links:
      - {rel: prior-art, to: prior/graphql}
  extracted:
    title: Queries quoted in host code, extracted and registered at build
    links:
      - {rel: prior-art, to: prior/lang-links}
---
Registered queries are what a schema change rechecks. What identifies
one decides how a client calls it and what a recheck reports.

## lang/registered/named

```
query identity_by_email (e: Text) -> Set Identity =
  from x in identities.EmailPass where x.email == e select x
client: call identity_by_email("a@b.c")
```

A stored procedure without effects. A failed recheck names the query.

## lang/registered/hashed

Apollo persisted queries: the client sends a hash, the server holds the
text. A failed recheck names a hash.

## lang/registered/extracted

```
let q = <@ fun e -> from x in identities.EmailPass where x.email == e select x @>
```

T-LINQ quotation; sqlx checks Rust-embedded SQL at build time
(https://github.com/launchbadge/sqlx). The build registers each quoted
query.

---
title: What identifies a schema version
pick: one
raised-by: [evolution/smo]
options:
  counter:
    title: A per-database counter incremented by each migration
    links:
      - {rel: prior-art, to: prior/surface-fdb-record-layer}
  hash:
    title: A content hash of the operators and the parent version
    links:
      - {rel: prior-art, to: prior/surface-gel-migrations}
---
Registered queries, client bundles and readers-first gating all name
the version they were checked against.

## surface/version/counter

```
schema version 8 = 7 + migration 0008
```

FDB Record Layer metadata version. Two databases at version 8 can
differ.

## surface/version/hash

```
migration m1q3k onto m1a9z { add constructor Identity.Apple { sub: Text } }
```

The same id means the same schema on every database, so a client
bundle compiled in CI can be checked against production by id.

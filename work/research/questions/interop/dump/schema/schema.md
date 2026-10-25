---
title: What a logical dump records of the schema
pick: one
raised-by: [interop/dump/logical]
options:
  op-log:
    title: The full operator log, replayed onto an empty database
    links:
      - {rel: gains, to: surface/source/op-log}
  folded:
    title: The current schema as one declaration set, with stable ids stated
---
The schema is the fold of its operator log (`surface/source/op-log`)
and registered bundles pin versions of it
(`surface/reader-tracking/bundles`).

## interop/dump/schema/op-log

```
schema
  0001 add type User = { name: Text }
  0002 add relation users : User key id
  ...
  0042 rename constructor Identity.Facebook to Meta
data identities.Google ...
```

Restore replays operators against empty relations, which is cheap.
Version numbers, constructor ids and bundle pins survive, and old
dumps restore on new engines. The log only grows.

## interop/dump/schema/folded

```
schema at 42 (5d2e...)
type Identity = EmailPass @1 { ... } | Meta @2 { ... } | Google @3 { ... }
```

Compact and readable. History before 42 is gone, so a bundle pinned
to version 40 cannot be translated after restore.

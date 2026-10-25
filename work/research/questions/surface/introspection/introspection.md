---
title: How the schema is introspected
pick: one
options:
  catalog-relations:
    title: The catalog is a set of relation-level sums in the same language
  api-only:
    title: Introspection through protocol calls returning descriptors
---
Tools need the schema: codegen, the REPL, migration diffing.

## surface/introspection/catalog-relations

```
type SchemaItem = SumDef { ... }
                | CtorDef { sum: Key catalog.SumDef, ... }
                | IndexDef { ... }
                | IndexCtor { index: Key catalog.IndexDef, ctor: Key catalog.CtorDef }
relation catalog : SchemaItem key name

from c in catalog.CtorDef where c.sum.name == "Identity" select c.name
```

The catalog is queried with match like user data. Gel exposes its
schema as `schema::` types.

## surface/introspection/api-only

A `describe` message returns the schema as a structure. Simpler, but
schema tooling cannot use the query language.

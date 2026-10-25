---
title: How the schema is stored
pick: one
raised-by: [substrate/new-engine]
options:
  catalog-relations:
    title: The catalog is sums in the engine, read and written by transactions
  metadata-document:
    title: One versioned schema value per database, replaced whole
    links:
      - {rel: prior-art, to: prior/engine-fdb-record-layer}
links:
  - {rel: prior-art, to: prior/engine-fdb-record-layer}
---
The catalog holds sums, constructors with their stable ids, indexes
with their constructor sets, constraints, schema versions with the
operators between them, and registered queries. Constructor ids come
from a catalog counter that never goes back, even when the transaction
allocating one aborts.

## engine/catalog-relations

Postgres system catalogs, written in the engine's own model. MVCC gives
each transaction the schema of its snapshot; DDL is ordinary writes.

```
type CatalogObject
  = Sum         { id: SumId, name: Text }
  | Constructor { id: CtorId, sum: SumId, name: Text, retired: Bool }
  | Index       { id: IndexId, sum: SumId, ctors: Set CtorId, key: [FieldId] }

relation catalog : CatalogObject key id

from c in catalog.Constructor where c.sum == s select c.name
```

## engine/metadata-document

FoundationDB Record Layer: one meta-data value whose version only
increases; each store header records the version it last saw; opening
a store with newer meta-data runs the steps between. Schema changes are
swaps of the whole value, checked by a validator.

```
schema v12 {
  type Identity = EmailPass #1 { ... } | Google #2 { ... } | Apple #4 { ... }
}
```

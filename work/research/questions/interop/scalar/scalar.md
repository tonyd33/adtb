---
title: Whether there are scalar types beyond the base set and newtypes
pick: one
options:
  newtype-only:
    title: Every type is a sum, record or newtype over the base types
    links:
      - {rel: gains, to: model/newtype}
  opaque:
    title: An extension may add an opaque type with its own encoding, order and text form
    links:
      - {rel: prior-art, to: prior/interop-pg-extensions}
---
Geometry, vectors, IP ranges and money are the usual requests. A
record of floats is a point; a polygon is a collection, and collections
are relations (`model/no-collections`).

## interop/scalar/newtype-only

```
type Point = { x: F64, y: F64 }
type Cidr  = new Bytes
```

Operators are functions in the query language or `interop/udf`.

## interop/scalar/opaque

```
extension postgis-like provides type Geometry
  encode, decode, compare, key_encode, parse, print
```

PostGIS `geometry`, pgvector `vector`
(https://github.com/pgvector/pgvector). The engine stores bytes it
cannot read.

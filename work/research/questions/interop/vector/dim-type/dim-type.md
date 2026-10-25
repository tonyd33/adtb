---
title: What type holds an embedding
pick: one
raised-by: [interop/vector/core-hnsw, interop/vector/ext, interop/vector/scan-only]
options:
  base-vector:
    title: A base type of fixed dimension and element type
  newtype-bytes:
    title: A newtype over Bytes with a domain constraint on its length
    links:
      - {rel: gains, to: model/newtype}
---
Collections are relations (`model/no-collections`), so an embedding
cannot be a list of `F32`.

## interop/vector/dim-type/base-vector

```
type Chunk = Para { text: Text, embedding: Vector F32 768 } | Code { ... }
```

A fixed-size value, not a collection: no element is a fact. Dimension
mismatch is a type error.

## interop/vector/dim-type/newtype-bytes

```
type Emb768 = new Bytes where length(self) == 3072
```

Distance functions decode bytes; dimension is checked on write
(`constraint/refine/write-only`).

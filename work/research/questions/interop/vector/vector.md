---
title: Where vector similarity search lives
pick: one
options:
  core-hnsw:
    title: An HNSW index kind over a constructor set in core
    links:
      - {rel: prior-art, to: prior/interop-usearch}
      - {rel: prior-art, to: prior/interop-pgvector}
  ext:
    title: An extension index kind, as pgvector
    links:
      - {rel: prior-art, to: prior/interop-pgvector}
  scan-only:
    title: A vector type and distance functions; exact search by scan only
  out:
    title: Out of scope
---
Nearest neighbours by embedding. pgvector offers HNSW and IVFFlat and
six distance operators (https://github.com/pgvector/pgvector).

## interop/vector/core-hnsw

```
add hnsw index (embedding) in chunks.(Para | Code) metric cosine m 16
top 10 by distance(c.embedding, q) (from c in chunks.(Para | Code) select c)
```

usearch is an HNSW library with Rust bindings and memory-mapped
serving (https://github.com/unum-cloud/usearch).

## interop/vector/ext

Through `interop/index-am`.

## interop/vector/scan-only

Exact top-k by a vectorized scan; fine to about a million rows
(unverified).

## interop/vector/out

Embeddings are bytes; search is elsewhere.

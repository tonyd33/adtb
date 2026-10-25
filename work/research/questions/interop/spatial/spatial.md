---
title: Where spatial indexing lives
pick: one
options:
  framework-ext:
    title: R-trees as an instance of the generalized search tree, shipped as an extension
    needs: [interop/index-am/framework]
  rtree:
    title: A core R-tree index kind over a core geometry type
  out:
    title: Out of scope; geohash or S2 cells as B-tree keys only
    links:
      - {rel: variant-of, to: interop/index-am/derived}
---
Range types and `constraint/overlap/range-index` already want a range
index; a 2-D R-tree is the same tree shape.

## interop/spatial/framework-ext

PostGIS indexes geometry through GiST (unverified detail).

## interop/spatial/rtree

```
add rtree index (location) in stores
from s in stores where s.location within box(...) select s
```

SQLite R-Tree is a virtual table (https://www.sqlite.org/vtab.html).

## interop/spatial/out

```
add index (geohash(location.lat, location.lon, 7)) in stores
```

Prefix ranges on a B-tree; proximity over cell boundaries is the
query's problem.

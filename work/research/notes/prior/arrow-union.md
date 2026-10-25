---
title: Apache Arrow union layouts
---
Dense union: int8 types buffer plus int32 offsets, children compact.
Sparse union: types buffer only, every child full length, easier to
vectorize. Neither has a top-level validity bitmap. Parquet has no union
logical type (PARQUET-756 open).
https://arrow.apache.org/docs/format/Columnar.html

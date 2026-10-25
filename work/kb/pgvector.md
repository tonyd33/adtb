# pgvector

kind: system
home: https://github.com/pgvector/pgvector

## Indexes

- HNSW (m default 16, ef_construction default 64; better queries, slower build) and IVFFlat (lists ~ rows/1000 up to 1M rows, sqrt(rows) beyond). [V 2026-10-25] https://github.com/pgvector/pgvector
- Operators: <-> L2, <#> inner product, <=> cosine, <+> L1, <~> Hamming, <%> Jaccard. [V 2026-10-25] same
- With approximate indexes filtering is applied after the index scan; iterative index scans (0.8.0) scan more until enough rows. [V 2026-10-25] same
- vector indexes up to 2000 dimensions, halfvec 4000, binary quantization by expression index to 64000. WAL-logged. [V 2026-10-25] same

# Velox (Pedreira et al., VLDB 2022)

kind: system
home: https://www.vldb.org/pvldb/vol15/p3372-pedreira.pdf

## Execution

- C++ library taking a fully optimized plan; no parser or optimizer; integrated with Presto, Spark, PyTorch and others at Meta. [V 2026-10-25] sec. 1-2
- Vectors are Arrow-compatible with encodings flat, dictionary, constant, sequence/RLE and bias; strings as 16-byte views to allow out-of-order writes; lazy vectors load on first use. [V 2026-10-25] sec. 4.1-4.2
- Kernels specialised for flat, constant and single-level dictionary inputs; nested dictionaries are flattened into a new index array. [V 2026-10-25] sec. 4.2
- Adaptivity: filters and conjuncts reordered at run time by measured selectivity and cost; hash table layout chosen adaptively. [V 2026-10-25] sec. 4.3, 4.5

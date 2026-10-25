---
title: PostgreSQL SSI lock manager internals
---
SIREAD locks on tuples, heap pages, relations, and B-tree leaf pages
for gaps; promoted to coarser levels under memory pressure; no intention
locks. A tuple lock does not follow the row's later versions (proved
unnecessary). An update moving a row into a scanned range must conflict.
Table rewrites promote tuple and page locks to relation locks; a dropped
index's gap locks become a heap relation lock. Committed transactions
are summarized into one dummy holder to bound memory.
https://raw.githubusercontent.com/postgres/postgres/master/src/backend/storage/lmgr/README-SSI
https://arxiv.org/abs/1208.4179

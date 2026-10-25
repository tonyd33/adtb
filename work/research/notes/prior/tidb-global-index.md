---
title: TiDB global index
---
GA in v8.4. Value holds handle plus partition id. DROP and TRUNCATE
PARTITION delete entries one by one, since a partition's entries are not
contiguous. https://docs.pingcap.com/tidb/stable/global-indexes/

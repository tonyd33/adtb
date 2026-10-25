---
title: PostgreSQL global index proposals
---
Unique constraints on partitioned tables must include the partition key.
Proposals: Ahmed and Akhtar 2019 (partition id as INCLUDE column),
HighGo 2022 (probe every partition's index on insert), Dilip Kumar
2025 (4-byte partition id per entry mapped through a catalog, never an
OID; detach invalidates the mapping; inserts lock the hierarchy; vacuum
must batch the global index pass, 15 s to 45 min unbatched on 1000
partitions). `_bt_check_unique` fetches through a single `heapRel`.
https://www.postgresql.org/message-id/CAFiTN-uyec_y2QS2whUam8Rp1M%2BPPGuP0Zz45uX_U6mhQ8mtRg%40mail.gmail.com

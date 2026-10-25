---
title: InnoDB buffer pool and fuzzy checkpoints
---
LRU with midpoint insertion: new pages enter the old sublist (3/8 of
the pool) and are promoted only when touched again after
innodb_old_blocks_time (1000 ms), so scans do not flush the hot set.
Fuzzy checkpointing flushes dirty pages in small batches; recovery
scans the redo log from the checkpoint label. Doublewrite files
against torn pages.
https://dev.mysql.com/doc/refman/8.4/en/innodb-performance-midpoint_insertion.html
https://dev.mysql.com/doc/refman/8.4/en/innodb-checkpoints.html

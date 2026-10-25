# WiredTiger

kind: system
home: https://source.wiredtiger.com

## Block manager

- No-overwrite: a modified page is written as a new block; a block is page header, block header (size, checksum, flags) and one B-tree page. [V 2026-10-25] https://source.wiredtiger.com/develop/arch-block.html
- Address cookie on the parent reference: offset and size in allocation units, optional object id, checksum; checksums are compared across cookie, header and buffer on read. [V 2026-10-25] same page
- allocation_size default 4 KB; free space as three extent lists per checkpoint (alloc, avail, discard); best-fit allocation from a size-sorted skiplist, first-fit for root pages. [V 2026-10-25] same page

## Cache

- eviction_target 80% (eviction threads active), eviction_trigger 95% (application threads evict, throttling), eviction_dirty_target 5%, eviction_dirty_trigger 20%. [V 2026-10-25] https://source.wiredtiger.com/develop/tune_cache.html
- Recovery starts from the last checkpoint and replays the log; old checkpoints dropped only when their blocks are unused. [V 2026-10-25] https://source.wiredtiger.com/develop/arch-checkpoint.html

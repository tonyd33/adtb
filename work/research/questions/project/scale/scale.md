---
title: Target data size on one node
pick: one
raised-by: [substrate/new-engine]
options:
  fits-ram:
    title: Working set and data fit in memory; disk is for durability
  out-of-core:
    title: Data many times larger than memory on one node
    links:
      - {rel: gains, to: buffer/cache/vmcache}
      - {rel: gains, to: buffer/io-path/direct}
---
Sets what the buffer pool, the page-id space and the single file must
hold. SQLite's ceiling is 4294967294 pages, about 281 TB at 64 KiB
pages (https://www.sqlite.org/limits.html).

## project/scale/fits-ram

Hekaton and TigerBeetle-as-hot-set shape: eviction is rare, page
latches and I/O scheduling matter less than CPU paths. The settled
vmcache and direct I/O stay correct but are tuned for the uncommon
case.

## project/scale/out-of-core

LeanStore and vmcache target this (`buffer/cache/vmcache`): working
set in memory, the rest on NVMe. Page-id width and file size limits
must cover tens of terabytes.

```
relation events : Event key id   -- 10^10 rows, 2 TB, 64 GB RAM
```

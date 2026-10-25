---
title: Checksummed block grids (TigerBeetle, WiredTiger)
---
TigerBeetle: one data file, a grid of 512 KiB copy-on-write blocks
referenced by (u64 address, u128 checksum) held in the referrer, so
misdirected and lost writes are detected; superblock in 4 copies.
WiredTiger: no-overwrite blocks in 4 KB allocation units, address
cookies carry offset, size and checksum; free space as alloc, avail and
discard extent lists per checkpoint.
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/data_file.md
https://source.wiredtiger.com/develop/arch-block.html

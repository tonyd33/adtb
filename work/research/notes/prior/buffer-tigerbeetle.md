---
title: TigerBeetle storage fault model, static allocation and VOPR
---
Assumes disks return corrupt data, misdirect I/O, lose writes, tear
sectors and turn slow. Everything is immutable, checksummed and
hash-chained; references carry the child's checksum; repair comes from
other replicas, and a record corrupt on every replica halts the
cluster. Own page cache, O_DIRECT, io_uring only, optional raw block
device. All memory allocated at startup, every queue bounded
(TIGER_STYLE). VOPR runs the real cluster deterministically on one
thread with injected storage, network and process faults.
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/concepts/safety.md
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md

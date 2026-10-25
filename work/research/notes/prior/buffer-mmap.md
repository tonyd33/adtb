---
title: mmap as a buffer pool (Crotty, Leis, Pavlo CIDR 2022; LMDB)
---
File-backed mmap fails four ways: the OS may write back a dirty page
before commit (mlock does not prevent it), any access may block on a
fault with no async reads, I/O errors arrive as SIGBUS anywhere, and
eviction does not scale (one kswapd, page table contention, TLB
shootdowns). Read-only random reads fell to about half of O_DIRECT
pread once the page cache filled; scans over 10 SSDs were about 20x
slower. MongoDB, InfluxDB and SingleStore dropped mmap.
LMDB is the standing counterexample: read-only map by default, writes
through a copy-on-write B+tree with one writer, two meta pages, no log.
https://db.cs.cmu.edu/papers/2022/cidr2022-p13-crotty.pdf
http://www.lmdb.tech/doc/group__mdb.html

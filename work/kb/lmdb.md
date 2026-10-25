# LMDB

kind: system
home: http://www.lmdb.tech/doc/

## Storage and concurrency

- One memory-mapped file holding a copy-on-write B+tree; MVCC readers never block; at most one writer at a time. [S 2026-10-25] http://www.lmdb.tech/media/20150921-SDC-LMDB.pdf
- No write-ahead log and no recovery step after a crash. [S 2026-10-25] http://www.lmdb.tech/media/20150921-SDC-LMDB.pdf

## Durability flags

- Two meta pages (NUM_METAS 2), each carrying the txnid that committed it. [V 2026-10-25] https://github.com/LMDB/lmdb/blob/mdb.master/libraries/liblmdb/mdb.c
- Map is read-only by default; MDB_WRITEMAP makes it writable at the cost of protection from wild pointer writes. MDB_NOMETASYNC skips the meta flush (ACI kept, last commit may be undone); MDB_NOSYNC skips flushes (crash can corrupt or lose transactions). [V 2026-10-25] http://www.lmdb.tech/doc/group__mdb.html

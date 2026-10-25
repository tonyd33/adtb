---
title: Embedded storage libraries
---
LMDB: one memory-mapped copy-on-write B+tree, MVCC readers, one writer
at a time, no write-ahead log, no recovery step.
http://www.lmdb.tech/media/20150921-SDC-LMDB.pdf
RocksDB: LSM; TransactionDB locks written keys, OptimisticTransactionDB
validates read and written keys at commit; snapshot isolation only.
https://github.com/facebook/rocksdb/wiki/Transactions
fjall (Rust LSM): keyspaces, optimistic SSI transactions since 2.2, or
a single-writer mode.
https://docs.rs/fjall/latest/fjall/
https://fjall-rs.github.io/post/fjall-2-2/

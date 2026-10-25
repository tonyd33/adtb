# RocksDB

kind: system
home: https://rocksdb.org/

## Transactions

- TransactionDB (pessimistic) locks keys on write; OptimisticTransactionDB tracks read and written keys and validates at commit. [S 2026-10-25] https://github.com/facebook/rocksdb/wiki/Transactions
- Isolation is snapshot; a snapshot is a sequence number. [S 2026-10-25] https://github.com/facebook/rocksdb/wiki/Transactions

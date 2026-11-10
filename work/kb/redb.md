# redb

kind: system
home: https://github.com/cberner/redb

## Transactions

- `WriteTransaction::commit(self)` and `abort(self)` consume the transaction; each sets a `completed` flag first so a failure inside them does not take the abort path again on drop. [V 2026-11-10] https://github.com/cberner/redb/blob/master/src/transactions.rs
- `Drop for WriteTransaction`: when not completed, not panicking and storage has not failed, it aborts and only logs an abort error (Drop cannot return it); when storage has failed it clears pending root updates without aborting; when a panic is unwinding it asserts the panic and marks the database as needing repair rather than running the abort. [V 2026-11-10] same
- A transaction is poisoned when an operation fails part way (a storage error in rename or delete of a table, a failed savepoint restore); commit then aborts and returns `CommitError::TransactionPoisoned`. [V 2026-11-10] same, and https://docs.rs/redb/latest/redb/struct.WriteTransaction.html

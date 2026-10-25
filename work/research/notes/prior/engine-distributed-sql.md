---
title: CockroachDB and TiDB isolation
---
CockroachDB: SERIALIZABLE by default, READ COMMITTED opt-in; clients
must retry restarted transactions.
https://docs.cockroachlabs.com/docs/stable/transactions
TiDB: snapshot isolation advertised as REPEATABLE READ, permits write
skew; READ COMMITTED also offered.
https://docs.pingcap.com/tidb/stable/transaction-isolation-levels/

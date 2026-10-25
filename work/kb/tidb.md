# TiDB

kind: system
home: https://docs.pingcap.com/

## Concurrency

- Snapshot isolation advertised as REPEATABLE-READ; no strict phantoms (A3) but broad phantoms (P3) and write skew allowed; READ COMMITTED also offered. [V 2026-10-25] https://docs.pingcap.com/tidb/stable/transaction-isolation-levels/

## Global index

- Experimental v8.3, GA v8.4. Key `tableID_indexID_cols`, value holds handle plus partitionID. DROP/TRUNCATE/REORGANIZE PARTITION wait for global index update; EXCHANGE PARTITION unsupported. [V 2026-10-25] https://docs.pingcap.com/tidb/stable/global-indexes/
- A partition's entries are not contiguous, so drop deletes entry by entry. [V 2026-10-25] https://github.com/pingcap/tidb/blob/master/docs/design/2020-08-04-global-index.md

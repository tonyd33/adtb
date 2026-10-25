# YugabyteDB

kind: system
home: https://docs.yugabyte.com/

## Indexes

- Secondary indexes on ordinary tables are global and sharded independently; on PG-style partitioned tables unique keys must include the partition key. [S 2026-10-25] https://www.yugabyte.com/blog/distributed-sql-essentials-sharding-and-partitioning-in-yugabytedb/

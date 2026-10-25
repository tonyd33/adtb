# IBM Db2

kind: system
home: https://www.ibm.com/docs/en/db2

## Partitioning

- Partition-level REORG makes the logical partition of a nonpartitioned index unavailable; online REORG rebuilds the whole NPI. [S 2026-10-25] https://robertsdb2blog.blogspot.com/2010/12/reorg-and-db2-for-zos-partition-by.html

## Typed tables

- Typed-table hierarchy stored as one hierarchy table with all subtable columns, typecode and OID columns. [U] https://www.columbia.edu/sec/acis/db2/db2s0/sqls0621.htm , USPTO 6421658

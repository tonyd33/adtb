---
title: Undo-based MVCC in InnoDB, Oracle and OrioleDB
---
InnoDB: clustered rows updated in place with a 6-byte trx id and 7-byte
roll pointer into undo; secondary entries delete-marked and purged;
secondary reads check the page's max trx id and fall back to the
clustered row. Oracle: per-block ITL, consistent-read block clones from
undo, delayed block cleanout writes on read, snapshot too old when
circular undo is reused. OrioleDB: index-organized Postgres engine,
row and block undo, CSN snapshots, 64-bit xids, no vacuum; primary-key
update is delete plus insert and a waiting updater errors.
https://dev.mysql.com/doc/refman/8.4/en/innodb-multi-versioning.html
https://docs.oracle.com/en/database/oracle/oracle-database/19/cncpt/data-concurrency-and-consistency.html
https://github.com/orioledb/orioledb/blob/main/doc/architecture/overview.mdx

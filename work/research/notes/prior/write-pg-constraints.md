---
title: Postgres deferrable constraints and ON CONFLICT
---
Only UNIQUE, PRIMARY KEY, REFERENCES and EXCLUDE are deferrable; CHECK
and NOT NULL are per row. ON CONFLICT accepts only non-deferrable
arbiters, may not touch a row twice, and may not move a row between
partitions. https://www.postgresql.org/docs/current/sql-set-constraints.html
https://www.postgresql.org/docs/current/sql-insert.html

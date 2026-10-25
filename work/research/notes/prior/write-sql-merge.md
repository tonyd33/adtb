---
title: SQL MERGE
---
Join source to target; `WHEN MATCHED`, `WHEN NOT MATCHED [BY TARGET]`,
`WHEN NOT MATCHED BY SOURCE` arms insert, update or delete. A target row
matched twice errors. Postgres docs steer concurrent upserts to
`INSERT ON CONFLICT`. https://www.postgresql.org/docs/current/sql-merge.html

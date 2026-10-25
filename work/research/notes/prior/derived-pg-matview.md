---
title: Postgres materialized views
---
REFRESH MATERIALIZED VIEW reruns the query and replaces the contents; CONCURRENTLY needs a unique index and does not block readers. Not incremental. https://www.postgresql.org/docs/current/sql-refreshmaterializedview.html

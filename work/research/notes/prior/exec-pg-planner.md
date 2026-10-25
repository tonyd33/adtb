---
title: Postgres partition pruning, plan cache and statistics
---
Pruning at plan time, executor init and per parameter change during
execution. Prepared statements: five custom plans, then generic if not
much costlier; replanned on DDL, statistics change, search_path.
Per-column MCV, histogram, n_distinct; `inherited` stats per hierarchy.
https://www.postgresql.org/docs/current/ddl-partitioning.html
https://www.postgresql.org/docs/current/sql-prepare.html
https://www.postgresql.org/docs/current/planner-stats.html

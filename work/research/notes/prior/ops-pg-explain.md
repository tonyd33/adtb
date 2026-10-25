---
title: Postgres EXPLAIN and auto_explain
---
EXPLAIN shows the chosen plan; ANALYZE runs the statement, side effects
included, and adds actual rows and timings per node; BUFFERS, WAL,
MEMORY, SERIALIZE, GENERIC_PLAN add detail; FORMAT text, json, xml,
yaml. auto_explain logs plans of statements slower than a threshold.
https://www.postgresql.org/docs/current/sql-explain.html
https://www.postgresql.org/docs/current/auto-explain.html

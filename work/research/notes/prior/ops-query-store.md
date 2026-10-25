---
title: SQL Server Query Store, plan forcing and automatic plan correction
---
Per-database store of query texts, plans, runtime and wait statistics in
time windows. A plan can be forced by id; forcing recompiles toward it
and records failures (NO_INDEX when an index it uses is gone).
Automatic plan correction forces the last good plan on a detected
regression and unforces it if it is not better.
https://learn.microsoft.com/en-us/sql/relational-databases/performance/monitoring-performance-by-using-the-query-store
https://learn.microsoft.com/en-us/sql/relational-databases/automatic-tuning/automatic-tuning

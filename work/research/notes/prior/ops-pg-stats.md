---
title: Postgres cumulative statistics, pg_stat_statements and wait events
---
pg_stat_* views over shared-memory counters, kept across clean restart,
reset after a crash, cached per transaction by default. pg_stat_statements
keys statements by a hash of the analysed tree with constants stripped,
bounded at 5000 entries by default. pg_stat_activity shows each
backend's wait_event_type (IO, Lock, LWLock, BufferPin, Client, IPC,
Timeout, Activity, Extension).
https://www.postgresql.org/docs/current/monitoring-stats.html
https://www.postgresql.org/docs/current/pgstatstatements.html

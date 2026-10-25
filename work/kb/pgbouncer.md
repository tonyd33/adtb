# PgBouncer

kind: system
home: https://www.pgbouncer.org/

## Pooling

- Pool modes: session (server connection for the client's whole connection), transaction (server connection only during a transaction), statement (transaction pooling with multi-statement transactions disallowed). [V 2026-10-25] https://www.pgbouncer.org/features.html
- Never work in transaction pooling: SET/RESET, LISTEN, WITH HOLD cursors, SQL PREPARE/DEALLOCATE, PRESERVE/DELETE ROWS temp tables, LOAD, session advisory locks. [V 2026-10-25] https://www.pgbouncer.org/features.html
- Protocol-level prepared statements work in transaction pooling only with `max_prepared_statements` non-zero. [V 2026-10-25] https://www.pgbouncer.org/features.html

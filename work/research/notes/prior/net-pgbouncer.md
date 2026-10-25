---
title: PgBouncer and Supavisor pooling
---
Session, transaction and statement pooling. Transaction pooling breaks
SET, LISTEN, WITH HOLD cursors, SQL PREPARE, temp tables and session
advisory locks; protocol prepared statements need
`max_prepared_statements`. Supavisor: multi-tenant transaction pooler
claiming a million client connections.
https://www.pgbouncer.org/features.html
https://github.com/supabase/supavisor

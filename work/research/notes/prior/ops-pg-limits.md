---
title: Postgres timeouts, cancellation and setting contexts
---
statement_timeout, lock_timeout, idle_in_transaction_session_timeout,
idle_session_timeout, transaction_timeout (17), all off by default.
pg_cancel_backend cancels a query; pg_terminate_backend ends a session.
Each setting has a context: internal, postmaster, sighup, backend,
superuser, user.
https://www.postgresql.org/docs/current/runtime-config-client.html
https://www.postgresql.org/docs/current/view-pg-settings.html

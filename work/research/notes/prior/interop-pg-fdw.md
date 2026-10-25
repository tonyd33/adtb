---
title: Postgres foreign data wrappers
---
Foreign tables are relations backed by a remote source; postgres_fdw pushes down filters, joins and writes. Constraints on foreign tables are not enforced locally and check constraints are not imported, since they may evaluate differently remotely.
https://www.postgresql.org/docs/current/postgres-fdw.html

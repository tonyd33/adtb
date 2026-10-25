# SQL Server

kind: system
home: https://learn.microsoft.com/en-us/sql/

## Partitioning

- Aligned indexes allow fast partition switch; non-aligned needed for unique keys without the partition column; building non-aligned needs more memory; unsupported over 1000 partitions. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/relational-databases/partitions/partitioned-tables-and-indexes?view=sql-server-ver16
- SWITCH requires aligned indexes except disabled nonclustered ones. [S 2026-10-25] https://learn.microsoft.com/en-us/previous-versions/sql/sql-server-2008-r2/ms191160(v=sql.105)

## Sparse columns

- Null costs nothing, non-null costs 4 extra bytes; needs 52 to 98 percent nulls to save 40 percent; not allowed in clustered or unique PK index; toggling sparse rewrites the table; filtered indexes recommended. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/relational-databases/tables/use-sparse-columns

## Query Store

- Query Store keeps a plan store, runtime stats store (aggregated per time window) and wait stats store (wait categories, since 2017) per database, written asynchronously; max_plans_per_query bounds plans. Enabled by default for new databases from SQL Server 2022. Plan forcing (sp_query_store_force_plan) needs no application change. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/relational-databases/performance/monitoring-performance-by-using-the-query-store
- Forcing recompiles the query and "typically produces exactly the same or a similar plan"; failures counted in force_failure_count with last_force_failure_reason, e.g. NO_INDEX (index no longer exists), NO_PLAN, HINT_CONFLICT, TIME_OUT. plan_forcing_type MANUAL or AUTO. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/sys-query-store-plan-transact-sql
- Automatic plan correction (FORCE_LAST_GOOD_PLAN) forces the last known good plan when a regression is detected (estimated CPU gain over 10 s, or more errors in the new plan), monitors it, unforces it if not better; a verified forced plan is kept until a recompile (statistics update or schema change). An unverified forcing is dropped on restart. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/relational-databases/automatic-tuning/automatic-tuning

## Temporal tables

- System-versioned temporal table = current table plus a history table with mirrored schema; two datetime2 period columns `GENERATED ALWAYS AS ROW START/END`; update/delete moves the old version to history; times are the transaction begin time (UTC); every modification adds a history row even if no value changes. SQL Server 2016+. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/relational-databases/tables/temporal-tables
- `FOR SYSTEM_TIME AS OF | FROM..TO | BETWEEN..AND | CONTAINED IN | ALL`, implemented as a union of current and history filtered by period; zero-length versions from multiple updates in one transaction are filtered out. [V 2026-10-25] same

## MARS

- Multiple Active Result Sets (SQL Server 2005): several pending requests and open default result sets per connection; off by default (`MultipleActiveResultSets=True`). Interleaving, not parallelism: switches only at yield points (SELECT, FETCH, RECEIVE); other statements run to completion. A transaction started in a batch under MARS must finish in the batch or is rolled back (batch-scoped transaction). Session impersonation prohibited while batches run. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/relational-databases/native-client/features/using-multiple-active-result-sets-mars

## Decimal arithmetic

- Result type rules: e1+e2 precision max(s1,s2)+max(p1-s1,p2-s2)+1, scale max(s1,s2); e1*e2 precision p1+p2+1, scale s1+s2; e1/e2 precision p1-s1+s2+max(6,s1+p2+1), scale max(6,s1+p2+1); capped at 38, with scale reduced (to no less than 6 in multiplication and division) or an overflow error. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/t-sql/data-types/precision-scale-and-length-transact-sql
- Predicate evaluation order is not guaranteed; aggregates inside CASE are evaluated before the CASE. [S 2026-10-25] https://learn.microsoft.com/en-us/archive/blogs/sqlprogrammability/predicate-ordering-is-not-guaranteed

## Row-level security

- Filter predicates silently filter SELECT, UPDATE, DELETE; block predicates (AFTER INSERT, AFTER UPDATE, BEFORE UPDATE, BEFORE DELETE) raise errors. Predicates are inline table-valued functions bound by CREATE SECURITY POLICY; with SCHEMABINDING ON, tables the predicate joins need no extra permission. Policies apply to dbo and db_owner too. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/relational-databases/security/row-level-security
- AFTER UPDATE block predicate is not checked if the columns it reads did not change. [V 2026-10-25] same
- Docs' side channel: `SELECT 1/(SALARY-100000) FROM PAYROLL WHERE NAME='John Doe'` reveals the salary through a divide-by-zero error despite the filter. DBCC SHOW_STATISTICS restricted since statistics cover unfiltered data; indexed views cannot sit on RLS tables since index lookups would bypass the policy; CDC and change tracking can leak filtered rows or keys; temporal history tables need their own predicate. [V 2026-10-25] same

## Ledger

- Ledger tables (updatable, with history table; or append-only): rows changed by a transaction are SHA-256 hashed in a Merkle tree, transactions hashed into a block, each block hashed with the previous block's root. The latest block hash (database digest) is stored outside the database (immutable blob storage, WORM); verification recomputes and compares. Ledger cannot prevent file edits by a machine administrator, only detect them at verification. [V 2026-10-25] https://learn.microsoft.com/en-us/sql/relational-databases/security/ledger/ledger-overview

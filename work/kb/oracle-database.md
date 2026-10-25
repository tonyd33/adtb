# Oracle Database

kind: system
home: https://docs.oracle.com/en/database/oracle/oracle-database/26/

## Assertions

- `CREATE ASSERTION name CHECK (...)` ships in Oracle AI Database 26ai, Release Update 23.26.1 (January 2026), first major engine to ship SQL assertions. [S 2026-10-25] https://fatdba.com/2026/03/29/oracle-26ai-assertions-the-database-constraint-that-took-years-to-ship/
- Two expression forms: existential `NOT EXISTS (SELECT ... WHERE ... [NOT] EXISTS (...))` and universal `ALL (SELECT ...) alias SATISFY (boolean_expression)`. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/26/sqlrf/create-assertion.html
- Not allowed in assertions: GROUP BY, HAVING, aggregate functions, outer joins, ANSI join syntax, scalar subqueries, set operators, WITH, analytic functions, PL/SQL functions (even deterministic), views, materialized views, quantified comparisons (ANY/SOME/ALL), JSON/LOB/ADT columns, row limits. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/26/sqlrf/create-assertion.html
- Non-deterministic functions (SYSDATE, SYSTIMESTAMP, SYS_CONTEXT, USER, ...) disallowed. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/26/sqlrf/create-assertion.html
- `[NOT] DEFERRABLE [INITIALLY IMMEDIATE | INITIALLY DEFERRED]`; immediate checks at end of statement, deferred at end of transaction. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/26/sqlrf/create-assertion.html
- Assertions supported only at READ COMMITTED; validation not supported at SERIALIZABLE. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/26/sqlrf/create-assertion.html
- Concurrency handled by assertion enqueues (locks) with modes SX, S, X; `USER_ASSERTION_LOCK_MATRIX` shows lock modes and scopes. ENABLE/DISABLE VALIDATE/NOVALIDATE states via ALTER ASSERTION. [S 2026-10-25] https://oracle-base.com/articles/26/assertions-26


## Constraints

- `CREATE ASSERTION` from release 23.26.1 (AI Database 26ai): cross-table constraints, deferrable initially deferred. [S 2026-10-25] https://blogs.oracle.com/sql/how-to-define-cross-table-constraints-with-assertions-in-oracle-ai-database
- Enforcement re-checks only changed rows; concurrency through AN locks at TABLE or JOIN_COLUMN_VALUES scope, shown in USER_ASSERTION_LOCK_MATRIX. [S 2026-10-25] https://oracle-base.com/articles/26/assertions-26

## Global indexes

- ROWID contains the data object number of the segment. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/19/sqlrf/Data-Types.html
- DROP PARTITION marks global indexes UNUSABLE unless `UPDATE INDEXES` or the partition is empty. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/19/vldbg/maintenance-partition-tables-indexes.html
- 12c asynchronous global index maintenance: DROP/TRUNCATE PARTITION metadata-only, orphaned entries cleaned by `SYS.PMO_DEFERRED_GIDX_MAINT_JOB` (daily 02:00), `DBMS_PART.CLEANUP_GIDX`, `ALTER INDEX ... COALESCE CLEANUP`; heap tables only. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/19/vldbg/maintenance-partition-can-be-performed.html
- Scans skip rowids of dropped segments; orphaned space not reclaimed by DML. [S 2026-10-25] https://richardfoote.wordpress.com/2013/08/06/12c-asynchronous-global-index-maintenance-part-ii-the-space-between/
- `ORPHANED_ENTRIES` column since 12.1.0.2. [S 2026-10-25] https://www.dbi-services.com/blog/oracle-12c-partitioning-enhancements-part-ii-other-improvements-1/

## Table clusters

- Tables sharing common columns stored in the same blocks; each cluster key stored once in cluster and cluster index; cluster index maps key to block address, must exist before inserts; rows of different tables in one block can share rowid. Avoid with frequent updates, full scans, truncates. No cross-table uniqueness. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/19/cncpt/tables-and-table-clusters.html

## Object types

- Substitutable object tables store a hierarchy in one table: hidden column per attribute added in any subtype, hidden typeid discriminant read by `SYS_TYPEID`. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/26/adobj/storage-of-objects.html
- `TREAT` returns NULL on type mismatch. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/19/sqlrf/TREAT.html
- REFs substitutable; `SCOPE IS` limits a REF to one object table; `IS OF (ONLY t)`. [S 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/19/adobj/Sql-object-types-and-references.html

## Undo and read consistency

- Changes write old values to undo segments; queries build consistent read (CR) clones of blocks as of their SCN: statement start under read committed, transaction start under serializable or read only. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/19/cncpt/data-concurrency-and-consistency.html
- Each block header holds an interested transaction list (ITL) pointing at undo segment transaction tables; row locks live in the data block, each locked row pointing at its ITL entry; INITRANS sizes the ITL. Oracle never escalates locks. [V 2026-10-25] same page
- Undo segments are circular; a long query whose undo was overwritten gets "snapshot too old"; undo retention sets a minimum keep time. [V 2026-10-25] same page
- Delayed block cleanout: blocks not cleaned at commit keep the XID in the ITL; the next reader looks up the commit SCN in the undo segment header, cleans the ITL and lock bytes, and generates redo, so a SELECT can write. [S 2026-10-25] https://jonathanlewis.wordpress.com/oracle-core/oc-3-transactions-and-consistency/

## SQL plan management

- A SQL plan baseline is a set of accepted plans the optimizer may use for a statement, stored as outlines able to reproduce each plan. A new plan not in the baseline is added to plan history unaccepted; evolution verifies it performs at least as well before accepting. Fixed plans are preferred; new plans are not used until declared fixed. If no baseline plan is reproducible (e.g. every plan used a dropped index) the optimizer uses a new cost-based plan. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/23/tgsql/overview-of-sql-plan-management.html

## Materialized views

- Refresh ON COMMIT (in the committing transaction, longer commit), ON DEMAND (COMPLETE, FAST, FAST_PCT, FORCE), ON STATEMENT (after each DML, rolled back with it, no MV logs, must be fast refreshable). FAST refresh needs materialized view logs on all detail tables (ROWID, SEQUENCE, INCLUDING NEW VALUES); aggregate MVs need COUNT(*); join-only MVs need each table's ROWID in the select list; `DBMS_MVIEW.EXPLAIN_MVIEW` says which refresh methods apply. [V 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/19/dwhsg/refreshing-materialized-views.html

## Virtual Private Database

- VPD policy functions (DBMS_RLS.ADD_POLICY) return a predicate applied to the statement; static policies assume one predicate for all users except SYS and EXEMPT ACCESS POLICY holders. Column-level VPD with sec_relevant_cols_opt => ALL_ROWS returns all rows with sensitive columns shown as NULL, SELECT only. [S 2026-10-25] https://docs.oracle.com/en/database/oracle/oracle-database/26/arpls/DBMS_RLS.html

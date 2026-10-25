---
title: SQL ASSERTION and Oracle 26ai assertions
---
`CREATE ASSERTION ... CHECK (...)` entered SQL-92; no major engine
shipped it until Oracle AI Database 26ai (RU 23.26.1, January 2026).
Oracle admits only `NOT EXISTS` and `ALL (...) SATISFY (...)` forms
over base tables, refuses aggregates, outer joins, set operators and
functions, supports deferral, and runs only at READ COMMITTED, guarded
by assertion locks. https://docs.oracle.com/en/database/oracle/oracle-database/26/sqlrf/create-assertion.html

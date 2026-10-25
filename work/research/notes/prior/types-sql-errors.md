---
title: SQL error ordering
---
Postgres: subexpression evaluation order is undefined, so WHERE x > 0 AND y/x > 1.5 may divide by zero; CASE orders evaluation except under constant folding and for aggregates inside it (https://www.postgresql.org/docs/current/sql-expressions.html sec. 4.2.14). SQL Server: predicate ordering is not guaranteed (https://learn.microsoft.com/en-us/archive/blogs/sqlprogrammability/predicate-ordering-is-not-guaranteed).

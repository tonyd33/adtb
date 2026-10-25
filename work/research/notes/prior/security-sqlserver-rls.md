---
title: SQL Server row-level security
---
Filter predicates silently filter reads; block predicates (after
insert, after update, before update, before delete) raise errors. One
predicate per table and operation. Docs show a divide-by-zero query
leaking a filtered salary; statistics, indexed views, CDC and change
tracking are listed as leaks.
https://learn.microsoft.com/en-us/sql/relational-databases/security/row-level-security

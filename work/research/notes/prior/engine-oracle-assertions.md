---
title: Oracle SQL assertions
---
Release 23.26.1 (AI Database 26ai). `CREATE ASSERTION` states a
cross-table constraint, deferrable and initially deferred. Enforcement
is incremental over changed rows, serialized by AN locks at TABLE or
JOIN_COLUMN_VALUES scope, listed in USER_ASSERTION_LOCK_MATRIX.
https://blogs.oracle.com/sql/assertion-locking-part-1-why
https://oracle-base.com/articles/26/assertions-26

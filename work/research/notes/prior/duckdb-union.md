---
title: DuckDB UNION type
---
Native tagged union column type with named members, up to 255 members.
Implemented on top of STRUCT with the tag first, so every member column
spans all rows. Value-level: members are not keys and cannot carry
constraints. https://duckdb.org/docs/2.0/sql/data_types/union.html

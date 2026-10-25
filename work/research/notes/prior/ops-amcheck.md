---
title: Postgres amcheck and SQLite integrity_check
---
amcheck verifies B-tree invariants, optionally that every heap tuple is
indexed, and heap structure, returning one row per corruption from
verify_heapam; it never repairs. SQLite integrity_check checks order,
record format, pages, index entries and constraints; quick_check skips
index-to-table matching.
https://www.postgresql.org/docs/current/amcheck.html
https://www.sqlite.org/pragma.html

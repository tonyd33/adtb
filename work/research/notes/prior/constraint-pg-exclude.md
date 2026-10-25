---
title: Postgres exclusion and temporal constraints
---
`EXCLUDE USING gist (room WITH =, during WITH &&)` generalizes unique
to an operator with index support (unverified wording). PostgreSQL 18
adds `WITHOUT OVERLAPS` on a key's last range column and `PERIOD`
foreign keys, both over GiST. CHECK sees only the row checked.
https://www.postgresql.org/about/news/postgresql-18-released-3142/

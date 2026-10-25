# Guagliardo and Libkin, A formal semantics of SQL queries (PVLDB 2017)

kind: paper
home: https://www.vldb.org/pvldb/vol11/p27-guagliardo.pdf

## Validation

- Paper semantics of basic SQL with NULLs and three-valued logic, implemented in Python only as an oracle, two variants (PostgreSQL, Oracle). [V 2026-10-25] PVLDB 11(1):27-39
- 100,000 random queries over a fixed all-int schema R1..R8, random data via Datafiller capped at 50 rows per table; outputs always matched PostgreSQL and Oracle. [V 2026-10-25] same

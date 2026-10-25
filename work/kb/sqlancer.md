# SQLancer (Rigger and Su test oracles)

kind: paper
home: https://github.com/sqlancer/sqlancer

## Pivoted Query Synthesis (PQS, OSDI 2020)

- PQS picks one random pivot row, generates random WHERE/JOIN expressions, evaluates them on the pivot row and rewrites each to yield TRUE; a result missing the pivot row is a bug (the containment oracle). [V 2026-10-25] Rigger, Su, "Testing Database Engines via Pivoted Query Synthesis", arXiv:2001.04174v1 (OSDI 2020 version)
- Argues differential testing across DBMS (Slutz's RAGS, 1998) is limited by dialect differences and misses bugs shared by all systems. [V 2026-10-25] same
- arXiv v1 reports 99 bugs (65 SQLite, 25 MySQL, 9 PostgreSQL), 61 via the containment oracle, 34 via internal errors, 4 crashes; PQS needs an expression evaluator per DBMS (high implementation effort, per the NoREC paper). [V 2026-10-25] same; arXiv:2007.08292

## NoREC (ESEC/FSE 2020)

- Non-Optimizing Reference Engine Construction: rewrite an optimizable query (predicate in WHERE) into one the DBMS cannot optimize (predicate evaluated per row in the select list, counted), then compare result sizes. Targets optimization bugs. [V 2026-10-25] Rigger, Su, "Detecting Optimization Bugs in Database Engines via Non-Optimizing Reference Engine Construction", arXiv:2007.08292
- Found 159 previously unknown bugs in PostgreSQL, MariaDB, SQLite, CockroachDB, 141 fixed, 51 optimization bugs. [V 2026-10-25] same

## Ternary Logic Partitioning (TLP, OOPSLA 2020)

- Query Partitioning: derive partitioning queries whose composed results must equal the original query's. TLP partitions on p, NOT p, p IS NULL (SQL three-valued logic); tests WHERE, GROUP BY, HAVING, aggregates, DISTINCT. [V 2026-10-25] Rigger, Su, "Finding Bugs in Database Systems via Query Partitioning", OOPSLA 2020, https://www.manuelrigger.at/preprints/TLP.pdf
- 175 bugs in MySQL, TiDB, SQLite, CockroachDB and others, 125 fixed, 77 logic bugs. [V 2026-10-25] same

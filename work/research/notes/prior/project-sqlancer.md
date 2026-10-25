---
title: SQLancer test oracles (PQS, NoREC, TLP)
---
Rigger and Su's oracles for logic bugs without a second DBMS. PQS
synthesises a query that must return a chosen pivot row. NoREC rewrites
a query so the optimizer cannot touch it and compares counts. TLP
partitions a predicate into p, NOT p, p IS NULL and checks the union
equals the original. Hundreds of bugs across SQLite, PostgreSQL, MySQL,
CockroachDB, TiDB.
arXiv:2001.04174 (OSDI 2020); arXiv:2007.08292 (ESEC/FSE 2020);
https://www.manuelrigger.at/preprints/TLP.pdf (OOPSLA 2020)

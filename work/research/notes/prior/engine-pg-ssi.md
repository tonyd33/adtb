---
title: Serializable Snapshot Isolation in PostgreSQL
---
Ports and Grittner, VLDB 2012, after Cahill, Rohm and Fekete, SIGMOD
2008. Snapshot reads plus SIREAD predicate locks (tuple, B-tree page,
relation) detect rw-antidependencies and abort a pivot of a dangerous
structure. Read-only deferrable transactions wait for a safe snapshot
and then never abort. Performance close to SI, ahead of S2PL on
read-heavy loads. Postgres docs advise SSI, explicit locks or a
materialized conflict row to make application-level consistency checks
safe; under SI they suffer write skew.
https://arxiv.org/abs/1208.4179
https://www.postgresql.org/docs/current/transaction-iso.html
https://www.postgresql.org/docs/current/applevel-consistency.html

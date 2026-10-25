---
title: Elle and Jepsen isolation checking
---
Elle infers an Adya dependency graph from client-observed histories,
using list-append workloads whose reads reveal version order, and
reports every non-predicate Adya anomaly; sound for trace-recoverable
histories. Jepsen pairs it with real processes, partitions and kills.
Hermitage is the hand-written precedent: fixed concurrency scenarios
per isolation level.
https://www.vldb.org/pvldb/vol14/p268-alvaro.pdf
https://github.com/ept/hermitage

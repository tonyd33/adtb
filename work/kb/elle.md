# Elle (Kingsbury and Alvaro, VLDB 2021)

kind: paper
home: https://github.com/jepsen-io/elle

## Checker

- Infers an Adya-style dependency graph between client-observed transactions and detects every Adya anomaly except predicate ones, with concise explanations. [V 2026-10-25] Kingsbury, Alvaro, "Elle: Inferring Isolation Anomalies from Experimental Observations", PVLDB 14(3), https://www.vldb.org/pvldb/vol14/p268-alvaro.pdf
- Chooses objects and operations (e.g. list append with unique elements) so reads reveal version history: recoverability (each observed version maps to the write that produced it) and traceability (the version order is visible), giving ww, wr and rw edges. Cycle detection is linear time. [V 2026-10-25] same
- Proven sound for trace-recoverable histories: every reported anomaly is present in every Adya history compatible with the observation. [V 2026-10-25] same
- Case studies found anomalies in TiDB (retry mechanism reapplying writes, G-single read skew), YugaByte DB (anti-dependency cycles after master crashes) and FaunaDB. [V 2026-10-25] same

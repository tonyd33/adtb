# Serializable Snapshot Isolation in PostgreSQL (Ports, Grittner)

kind: paper
home: https://arxiv.org/abs/1208.4179

## Contribution

- VLDB 2012, PVLDB 5(12):1850-1861. First production SSI, and first in a system without prior lock-based serializable; new predicate lock manager, bounded memory, read-only optimization. [S 2026-10-25] https://dl.acm.org/doi/10.14778/2367502.2367523
- Serializable performance slightly below SI, well ahead of S2PL on read-intensive workloads. [S 2026-10-25] https://arxiv.org/abs/1208.4179
- Builds on Cahill, Rohm, Fekete, SIGMOD 2008 (SSI). [U]
- Read-only DEFERRABLE transactions wait for a safe snapshot, then run without SSI overhead or abort risk. [U]

## Implementation

- Serializable costs less than 7% throughput relative to SI on the paper's benchmarks; serialization failure rate under 1% on evaluated workloads. [V 2026-10-25] https://arxiv.org/abs/1208.4179
- B+-tree SIREAD gap locks are at leaf-page granularity, next-key locking planned; no intention locks needed: writers check relation, page and tuple locks coarsest to finest. [V 2026-10-25] sect. 5.2.1
- Victim choice: abort only after a conflicting transaction commits, so an immediate retry cannot hit the same structure; if T2 and T3 both committed, abort T1. [V 2026-10-25] https://arxiv.org/abs/1208.4179
- Cleanup: SIREAD locks of a committed transaction dropped once all concurrent transactions finish; if only read-only transactions remain, all committed transactions' SIREAD locks can be dropped. Each node keeps the commit sequence number of the earliest transaction it has a conflict out to. [V 2026-10-25] sect. 6.1

## Replication

- Sect. 7.2: log-shipping replication does not give serializable behaviour for read-only transactions on standbys, since commit order need not match apparent serial order under SSI (read-only anomaly). Tracking standby read sets back to the primary was rejected as too costly. Plan: mark safe snapshots in the log stream; standby transactions use the latest safe snapshot (possibly stale), wait for the next (as DEFERRABLE), or run at a weaker level. [V 2026-10-25] https://arxiv.org/pdf/1208.4179

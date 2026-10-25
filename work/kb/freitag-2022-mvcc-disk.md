# Freitag, Kemper, Neumann: Memory-Optimized MVCC for Disk-Based Database Systems

kind: paper
home: https://www.vldb.org/pvldb/vol15/p2797-freitag.pdf

## Design

- PVLDB 15(11) 2022, Umbra. Pages hold only the newest version; all other version information lives only in memory and is never persisted. Append-only (Postgres, Hekaton) persists redundant versions, causing write amplification. [V 2026-10-25] https://www.vldb.org/pvldb/vol15/p2797-freitag.pdf sect. 3.1
- A versioned page gets an in-memory local mapping table (stable tuple id to version chain head), pointed to from its buffer frame and reached under the page latch; evicted pages' tables kept in an orphan hash table and reattached on load. Unversioned pages (most) scan without lookups. [V 2026-10-25] sect. 3.1.1
- GC: Steam; transaction version buffers reclaimed at commit; empty mappings pruned on page access when over a threshold (e.g. 5%) of a table's chains are empty; orphaned tables pruned during buffer IO. [V 2026-10-25] sect. 3.1.2
- Recovery needs no version information: after redo no transaction is active, so all chains are empty. Rollback overwrites the page with the log's before-image and unlinks the version. [V 2026-10-25] sect. 3.1.3
- Out-of-memory fallback: bulk writers get exclusive write access (readers continue), create virtual versions marked by a per-page reference bulk-load epoch and per-tuple flags; a central persistent bulk epoch makes them visible at commit; one bulk operation at a time. [V 2026-10-25] sect. 3.2
- Throughput up to an order of magnitude above competing disk-based systems. [V 2026-10-25] abstract

# FoundationDB

kind: system
home: https://apple.github.io/foundationdb/

## Concurrency

- Global ACID transactions, strict serializability, MVCC reads and optimistic concurrency for writes; neither reads nor writes block. [V 2026-10-25] https://apple.github.io/foundationdb/developer-guide.html
- A transaction is conflict free iff no key it read was written between its read version and commit. [V 2026-10-25] https://apple.github.io/foundationdb/developer-guide.html
- Transactions over five seconds are not supported; keys above 10 kB rejected, values capped at 100 kB. [V 2026-10-25] https://apple.github.io/foundationdb/developer-guide.html
- Transaction size limited to 10 MB, counting written keys and values and explicitly added conflict range keys. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Client buffers writes until commit; read-your-writes is served by merging buffered writes with database reads. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Atomic operations (add, min, max, bitwise ops) and set-versionstamp write without reading, so they add no read conflict. [V 2026-10-25] https://apple.github.io/foundationdb/developer-guide.html
- Snapshot reads add no read conflict range, weakening isolation per read. [V 2026-10-25] https://apple.github.io/foundationdb/developer-guide.html
- The Sequencer hands out read and commit versions; commit versions advance at about one million per second and also serve as log sequence numbers, which with conflict checking gives strict serializability. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Resolvers are range-partitioned over the key space and run a lock-free check of each transaction's read ranges against recently committed writes; a transaction commits only if all Resolvers admit it; one single-threaded Resolver handles about 280K TPS. Resolvers keep only the 5-second MVCC window of history, which is why transactions are capped at 5 s. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf

## Architecture

- Unbundled: a control plane (Coordinators form a Paxos group, elect a ClusterController, which recruits Sequencer, DataDistributor, Ratekeeper) and a data plane split into a transaction system (Sequencer, Proxies, Resolvers; all stateless), a log system (LogServers, replicated persistent queues holding the WAL per StorageServer) and a storage system (StorageServers holding key-range shards). [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Commit path: Proxy gets commit version from Sequencer, sends read/write sets to Resolvers, then writes mutations to LogServers tagged by destination StorageServer; commit is acknowledged once all log replicas persist it. StorageServers pull redo logs from LogServers in the background, off the commit path. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Storage server engine (paper era): unversioned SQLite B-tree plus in-memory multi-versioned redo data; only mutations leaving the 5 s MVCC window are written to SQLite. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Configurable engines today: ssd (B-tree), memory (in-memory, logged to disk), ssd-redwood-1 (newer B-tree), ssd-rocksdb-v1 (LSM). Redundancy modes: single, double, triple, three_data_hall, three_datacenter. [V 2026-10-25] https://apple.github.io/foundationdb/configuration.html

## Recovery

- No ARIES-style checkpoint or redo replay on recovery: the new Sequencer locks coordinated state, stops old LogServers, recruits a new transaction system, and only determines the recovery version (end of redo log); StorageServers discard in-memory versions above it and keep applying logs asynchronously. Typical recovery under five seconds. Each such generation is an epoch. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Fail fast: any failure in the transaction system triggers the same whole-transaction-system recovery rather than per-failure handling. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf

## Data model and layers

- Core is an ordered map of byte-string keys to byte-string values; operations are get, set, getRange, clear (single key, range or prefix). Everything else (tables, indexes, documents) is a stateless layer above. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Data-modeling guide: indexes are extra key orderings written in the same transaction as the primary data; multimaps put values in keys with empty values; hierarchies use materialised paths. Keys ideally under 32 bytes, values under 10 kB. [V 2026-10-25] https://apple.github.io/foundationdb/data-modeling.html
- Subspaces namespace keys by a tuple prefix; the directory layer maps paths to short allocated prefixes so keys stay short and directories can be moved or renamed cheaply. [V 2026-10-25] https://apple.github.io/foundationdb/developer-guide.html
- Tuple layer is an order-preserving, self-describing encoding where each element starts with a type code: 0x00 null, 0x01 bytes, 0x02 unicode string, 0x05 nested tuple, 0x0c to 0x1c integers (by length and sign), 0x20 float, 0x21 double, 0x26 false, 0x27 true, 0x30 UUID, 0x33 96-bit versionstamp. Inside nested tuples null is escaped as \x00\xff. [V 2026-10-25] https://github.com/apple/foundationdb/blob/main/design/tuple.md
- Tuple type codes 0x40 to 0x4f are reserved for third-party extension types and will never be reused for standard types. [V 2026-10-25] https://github.com/apple/foundationdb/blob/main/design/tuple.md
- Because the type code leads, tuple keys of different element types sort by type code first; floats sort negative NaN, -inf, reals, +inf, positive NaN. [U] (type-code-first ordering inferred from encoding) https://github.com/apple/foundationdb/blob/main/design/tuple.md

## Testing

- Deterministic simulation: all code is written in Flow, an actor/async extension to C++; network, disk, time and RNG are abstracted so a whole cluster runs as one single-threaded discrete-event simulation; production is a thin shim over syscalls. Multithreaded concurrency is avoided (one process per core). [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Simulator injects machine, rack and datacenter failures, network partitions and latency, corruption of unsynced writes on reboot; "buggification" points in the code inject unusual but legal behaviour (spurious errors, delays, odd tuning values). Swarm testing randomises cluster, workloads, faults and enabled buggify points per run. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Simulation cannot find performance bugs or test code outside Flow, so FDB avoids external dependencies. [V 2026-10-25] Zhou et al., "FoundationDB: A Distributed Unbundled Transactional Key Value Store", SIGMOD 2021, https://www.foundationdb.org/files/fdb-paper.pdf
- Swizzle-clogging: pick a random subset of nodes, clog their connections one by one, then unclog in random order. [V 2026-10-25] https://apple.github.io/foundationdb/testing.html

## Tuple layer encoding

- Each tuple element is a type code byte then payload; tuples order by code first. Codes: 0x00 null, 0x01 bytes, 0x02 UTF-8 string, 0x05 nested tuple, 0x0c-0x1c integers (0x14 zero; 0x15..0x1c positive 1-8 bytes big-endian; 0x13..0x0c negative, one's complement), 0x20 float, 0x21 double, 0x26 false, 0x27 true, 0x30 UUID (16 bytes), 0x33 96-bit versionstamp, 0x40-0x4f user type codes, 0xff reserved as escape. [V 2026-10-25] https://raw.githubusercontent.com/apple/foundationdb/main/design/tuple.md
- Byte and string payloads end with 0x00; embedded 0x00 is escaped as 0x00 0xff, order-preserving. Nested tuple: 0x05, elements, 0x00; a null inside is 0x00 0xff. The deprecated 0x03..0x04 nested encoding misordered the empty tuple against (null). [V 2026-10-25] same
- Floats: big-endian IEEE; if sign bit set flip all bits, else flip sign bit; yields IEEE total order with -0.0 < 0.0 and NaNs at both ends. [V 2026-10-25] same
- Integer -5551212 packs as 0x11 0xab 0x4b 0x93; -1 as 0x13 0xfe. [V 2026-10-25] same

## Simulation

- Simulation runs a whole cluster deterministically in one single-threaded process on Flow (C++ coroutine runtime with production and simulated execution); tens of thousands of runs nightly; estimated one trillion CPU-hours total. [V 2026-10-25] https://apple.github.io/foundationdb/testing.html

## Replication and regions

- Multi-region: primary and standby regions each with a DC and satellites; sync or async cross-region replication configurable; design keeps commits in-region by logging to satellites, fails over automatically between regions, and loses data only on simultaneous total region failure. [V 2026-10-25] Zhou et al. SIGMOD 2021, sect. on geo-replication https://www.foundationdb.org/files/fdb-paper.pdf
- Read-only transactions are serializable at their read version and need no commit; MVCC window is short (5 seconds). [V 2026-10-25] same

## Client

- Retry loop: catch the error and call `on_error()`; `@fdb.transactional` wraps it. not_committed, transaction_too_old and future_version mean the transaction definitely failed and can be retried. commit_unknown_result: it either committed or not, and if not it never will; retry safely only with idempotency (ids generated outside the loop, checked inside). Limits 5 s and 10 MB. [V 2026-10-25] https://apple.github.io/foundationdb/developer-guide.html
- Error codes not_committed 1020, commit_unknown_result 1021, transaction_too_old 1007. [U]

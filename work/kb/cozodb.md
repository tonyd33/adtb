# CozoDB

kind: system
home: https://docs.cozodb.org

## Status

- Latest release v0.7.6, 2023-12-11; no release since. MPL-2.0. [V 2026-10-25] https://github.com/cozodb/cozo/releases
- Self-description: "general-purpose, transactional, relational database that uses Datalog for query, is embeddable", focused on graph data; argues the relational model "can deal with graph data just fine". [V 2026-10-25] https://github.com/cozodb/cozo

## Types

- Ten runtime types: Null, Bool, Number (Int i64 or Float f64, Int promotes to Float), String, Bytes, Uuid, List (mixed types, nested), Vector (F32/F64 fixed length), Json, Validity. No tagged union, variant or sum type. [V 2026-10-25] https://docs.cozodb.org/en/latest/datatypes.html
- Total cross-type order: Null < Bool < Number < other types; `-1 == -1.0`; lists lexicographic. [V 2026-10-25] https://docs.cozodb.org/en/latest/datatypes.html
- Column types: Int, Float, Bool, String, Bytes, Uuid, Json, Validity; `T?` nullable; `[T]` and `[T; n]` homogeneous lists; `(T1, T2, ...)` tuples; `<F32; n>` vectors; `Any` (all but null) and `Any?`. An omitted column type defaults to `Any?`. [V 2026-10-25] https://docs.cozodb.org/en/latest/datatypes.html https://docs.cozodb.org/en/latest/stored.html

## Stored relations

- Columns before `=>` form the composite key, after it the values; rows stored in trees sorted lexicographically by key. [V 2026-10-25] https://docs.cozodb.org/en/latest/stored.html
- Mutation ops: `:create`, `:put` (upsert by key), `:insert` (error if key exists), `:update` (partial non-keys; error if key absent), `:rm` (no-op if absent), `:delete` (error if absent), `:replace` (overwrite whole relation; "The schema of the replaced relation need not match"), `:ensure` / `:ensure_not` (assert rows present/absent at commit). [V 2026-10-25] https://docs.cozodb.org/en/latest/stored.html
- Triggers via `::set_triggers` with `on put` / `on rm` / `on replace`, seeing implicit `_new[]` and `_old[]` relations; "Triggers do not propagate": trigger writes do not fire further triggers. [V 2026-10-25] https://docs.cozodb.org/en/latest/stored.html
- System ops: `::relations`, `::columns`, `::remove`, `::rename`, `::access_level`, `::index`, `::hnsw`, `::running`, `::kill`, `::explain`, `::set_triggers`. No op adds, drops or alters a column; schema change is `:replace` or create-and-copy. [V 2026-10-25] https://docs.cozodb.org/en/latest/sysops.html
- Access levels per relation: `normal`, `protected` (no `::remove`/`:replace`), `read_only` (no mutation or triggers), `hidden` (no data access). [V 2026-10-25] https://docs.cozodb.org/en/latest/sysops.html

## Storage and indexes

- Pluggable storage: in-memory, SQLite, RocksDB, Sled, TiKV. [V 2026-10-25] https://github.com/cozodb/cozo
- Storage trait: `transact(write)`, `get`/`multi_get` with `for_update` (commit must fail if key modified outside the tx), `put`, `del`, `range_scan` (lower inclusive, upper exclusive), `range_skip_scan_tuple` for validity queries; `commit()` "Must return an Err if MVCC consistency cannot be guaranteed". No automatic retry. [V 2026-10-25] https://github.com/cozodb/cozo/blob/main/cozo-core/src/storage/mod.rs
- Whole database is one ordered KV space: tuple key = 8-byte big-endian `RelationId` (u64) prefix then memcmp-encoded key columns; non-key columns serialized as the value. `RelationId(0)` is SYSTEM and holds serialized `RelationHandle`s (name, id, keys/non_keys metadata, triggers, access_level, is_temp, indices). [V 2026-10-25] https://github.com/cozodb/cozo/blob/main/cozo-core/src/runtime/relation.rs
- "Indices are simply reordering of columns of the original stored relation"; each index is its own relation named `base:index`. Planner uses one only when it avoids a full scan. [V 2026-10-25] https://docs.cozodb.org/en/latest/stored.html https://github.com/cozodb/cozo/blob/main/cozo-core/src/runtime/relation.rs
- `::hnsw create rel:idx {dim, m, dtype, fields, distance, ef_construction, filter, ...}`, `::fts create`, `::lsh create` (MinHash); queried as `~rel:idx{...}`. `filter` restricts which rows are indexed (partial index). [V 2026-10-25] https://docs.cozodb.org/en/latest/vector.html
- HNSW indices protected under concurrent writes by MVCC. [V 2026-10-25] https://github.com/cozodb/cozo

## Transactions

- Each script is one transaction; several `{...}` queries in one script run serially in it. Transaction reads a snapshot of committed data. Ephemeral relations live only for the transaction. [V 2026-10-25] https://docs.cozodb.org/en/latest/stored.html
- `:assert none` / `:assert some` query options abort the transaction on unexpected results. [V 2026-10-25] https://docs.cozodb.org/en/latest/queries.html

## Time travel

- A relation is time-travel eligible if the last key column has explicit type `Validity` = (signed integer timestamp, boolean assert flag). Row valid from its timestamp until the next row's; `false` flag only ends the previous fact. Recommended unit microseconds since epoch, user-chosen. Valid time only; not bitemporal. [V 2026-10-25] https://docs.cozodb.org/en/latest/timetravel.html
- Validity sorts descending on timestamp then flag, so a seek lands on the latest row at or before the query time. Query with `*rel{...  @ <const>}`; `@` must be compile-time constant. `'ASSERT'` / `'RETRACT'` strings mean current timestamp. [V 2026-10-25] https://docs.cozodb.org/en/latest/timetravel.html

## Query language (CozoScript)

- Three rule kinds: inline `:=`, fixed `<~` (named algorithm/utility, e.g. graph algorithms), constant `<-`. Stored relations accessed by `*rel{col: var}` named binding or positional. [V 2026-10-25] https://docs.cozodb.org/en/latest/queries.html
- Set semantics for relations; aggregations use bag semantics. Multiple same-arity rule definitions form a union; `or` in bodies. [V 2026-10-25] https://docs.cozodb.org/en/latest/queries.html
- Negation safety: a negated atom needs a binding bound positively elsewhere. Self-recursion through aggregation allowed only for semi-lattice aggregations. [V 2026-10-25] https://docs.cozodb.org/en/latest/queries.html
- Execution: rule graph SCCs checked for stratification (abort if unstratifiable), magic-set rewrite, semi-naive bottom-up evaluation, compiler reorders body atoms; stored-relation cost depends on bound vars forming a key prefix; `:limit` stops early. [V 2026-10-25] https://docs.cozodb.org/en/latest/execution.html

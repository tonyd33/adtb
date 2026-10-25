# XTDB

kind: system
home: https://docs.xtdb.com

## Types

- Schema-on-read: columns need no declared type, but every column has one, inferred from written data and widened as different data arrives. [V 2026-10-25] https://docs.xtdb.com/about/types-in-xtdb.html
- Heterogeneous values make a union type, not a coercion: a column given `21`, `'sensor offline'` and no value has type `#{:i64 :utf8 :null}` ("a `BIGINT` or a `VARCHAR`"). [V 2026-10-25] https://docs.xtdb.com/about/types-in-xtdb.html
- Missing value and explicit NULL are equivalent; `:null` is a type; `[:? :utf8]` is string-or-NULL; `:nothing` is the bottom type of a column with no values yet; the top type is unnamed. [V 2026-10-25] https://docs.xtdb.com/about/types-in-xtdb.html
- Type join recurses into structs key by key (a key missing on one side becomes nullable) and into lists/sets element-wise (one collection type with a union element type). [V 2026-10-25] https://docs.xtdb.com/about/types-in-xtdb.html
- Arrow mapping: unions are Arrow dense unions with child fields named by Arrow type; a nullable union gets an extra `null` child; a single type plus NULL is a nullable field, not a union. Union legs are keyed by physical type, never by a user tag. [V 2026-10-25] https://docs.xtdb.com/about/types-in-xtdb.html
- Scalar types: SMALLINT/INT/BIGINT, FLOAT/DOUBLE, DECIMAL (non-32/64 precisions stored as 128/256 bit), DATE, TIMESTAMP, TIME, DURATION, INTERVAL, PERIOD, BOOLEAN, VARBINARY, VARCHAR, URI, UUID; native nested ARRAY and OBJECT (`{name: 'Lucy', age: 38}`), not JSON. [V 2026-10-25] https://docs.xtdb.com/reference/main/data-types.html

## Data model

- A database is a collection of tables grouped into schemas (default `public`); one database shares one transaction log and object storage. Primary database `xtdb`; secondaries via `ATTACH DATABASE ... WITH $$ log: ... $$`, `DETACH DATABASE` (async from v2.2). [V 2026-10-25] https://docs.xtdb.com/about/dbs-in-xtdb.html
- Transactions touch one database only; queries may span databases with `database.schema.table` names. [V 2026-10-25] https://docs.xtdb.com/about/dbs-in-xtdb.html
- Rows are documents that must carry `_id`; `_id` cannot be updated. INSERT on an existing `_id` upserts. No CREATE TABLE, primary key, unique or foreign key DDL on the DML page; tables appear on first write. [V 2026-10-25] https://docs.xtdb.com/reference/main/sql/txs.html
- PATCH upserts merging at key granularity: present keys override, absent or null keys keep existing values; accepts FOR PORTION OF VALID_TIME. [V 2026-10-25] https://docs.xtdb.com/reference/main/sql/txs.html

## Bitemporality

- Every table is bitemporal: system-maintained read-only `_system_from`/`_system_to` and user-controlled `_valid_from`/`_valid_to`; null `_to` means open. [V 2026-10-25] https://docs.xtdb.com/about/time-in-xtdb.html
- Default query basis: system time as best known, valid time as of now, so plain SQL looks atemporal. SQL:2011 clauses `FOR VALID_TIME AS OF`, `FOR ALL VALID_TIME`, `FOR SYSTEM_TIME AS OF`, `UPDATE ... FOR PORTION OF VALID_TIME FROM a TO b`; periods closed-open. [V 2026-10-25] https://docs.xtdb.com/about/time-in-xtdb.html
- INSERT/UPDATE/DELETE default valid range: now to end of time. ERASE removes documents for all valid time and all system time, irrevocably (GDPR). [V 2026-10-25] https://docs.xtdb.com/reference/main/sql/txs.html
- `FOR VALID_TIME ... ONLY` clamps projected `_valid_from`/`_valid_to` to the window instead of only filtering. [V 2026-10-25] https://docs.xtdb.com/reference/main/sql/queries.html

## Query languages

- SQL is primary, Postgres wire compatible. [V 2026-10-25] https://xtdb.com/blog/launching-xtdb-v2
- SQL extensions: FROM-first queries with any number of WHERE/GROUP BY/SELECT stages evaluated in order; SELECT may follow GROUP BY; SELECT optional (defaults `*`); FROM optional (0-column 1-row table); GROUP BY inferred from column refs outside aggregates; `NEST_ONE`/`NEST_MANY` subqueries return nested objects/arrays. [V 2026-10-25] https://docs.xtdb.com/reference/main/sql/queries.html
- XTQL: data-oriented composable language from Datalog and relational algebra, built for programmatic query generation. Source ops `from`, `rel`; tail ops `where`, `order-by`, `limit`, `with`, `aggregate`, `return`, `without`; pipelines chain one source through tails. [V 2026-10-25] https://docs.xtdb.com/xtql/tutorials/introducing-xtql.html
- XTQL joins by unification: a logic variable repeated within a `unify` clause must bind the same value everywhere. XTQL embeds in SQL as `XTQL $$ ... $$`. [V 2026-10-25] https://docs.xtdb.com/xtql/tutorials/introducing-xtql.html

## Concurrency and transactions

- Single writer: writes serialized through a totally ordered durable log; processing is serial, deterministic and atomic on the leader for a database; other nodes apply resolved results. Serializable isolation; transactions reified. [V 2026-10-25] https://docs.xtdb.com/concepts/key-concepts.html https://xtdb.com/blog/launching-xtdb-v2
- Write transactions are non-interactive: no SELECT in a DML transaction (errors). ASSERT rolls the transaction back when its predicate is false, the stated mechanism for constraints and concurrent safety checks. COMMIT takes SYNC/ASYNC. [V 2026-10-25] https://docs.xtdb.com/about/txs-in-xtdb.html https://docs.xtdb.com/reference/main/sql/txs.html
- Every query runs at a basis (snapshot plus clock); a connection sees its own submitted transactions; `AWAIT_TOKEN` for cross-connection visibility; `SNAPSHOT_TOKEN` for repeatable reads. [V 2026-10-25] https://docs.xtdb.com/about/txs-in-xtdb.html

## Storage and indexes

- Three layers: transaction log (e.g. Kafka), shared object storage (S3 etc.), compute nodes with caches. Files immutable and deterministic. [V 2026-10-25] https://xtdb.com/blog/building-a-bitemp-index-3-storage
- Primary index is an LSM on object storage of Arrow files: transactions land in L0, nodes compact into L1, L2...; compaction never discards superseded versions; a GC process deletes files superseded by compaction. [V 2026-10-25] https://xtdb.com/blog/dev-diary-feb-24
- Live (in-memory) index is a hash trie on entity-id with 4-way branching (2-bit prefixes), leaf pages split at 1024 items. L1 to L2+ compaction shards: four L1s become four L2s, each filtered by entity-id hash prefix. Target ~100MB per shard. [V 2026-10-25] https://xtdb.com/blog/building-a-bitemp-index-3-storage
- Recency (max timestamp at which a version was believed valid) splits data at L1 into a current path sharded by entity id and a historical path sharded by entity id then recency; queries skip files whose recency precedes the query time. File metadata has min/max and bloom filters. [V 2026-10-25] https://xtdb.com/blog/building-a-bitemp-index-3-storage
- No user secondary indexes: per-page column metadata only, under-selective. Proposed `CREATE INDEX`: one hash-keyed LSM per index fed from data L0, emitting candidate IIDs that the bitemporal resolver confirms ("the index proposes, the store disposes"). Selected for 2.x. [V 2026-10-25] https://github.com/xtdb/xtdb/issues/3663
- Index format changes are breaking and migrated by writing a new versioned root (`/v1`) in the object store. [V 2026-10-25] https://xtdb.com/blog/dev-diary-feb-24

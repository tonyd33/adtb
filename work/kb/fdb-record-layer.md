# FoundationDB Record Layer

kind: system
home: https://foundationdb.github.io/fdb-record-layer

## Evolution

- Metadata has an overall version incremented on any change, plus per record type and index versions recording when they were added or changed. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/SchemaEvolution.html
- `MetaDataEvolutionValidator` throws `MetaDataException` if new metadata needs stored data migrated; by default it rejects changes needing index rebuilds. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/SchemaEvolution.html
- Adding record types is always allowed; removing fields means deprecating them; renames need index and primary key definitions updated, since serialization uses field numbers in the union descriptor. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/SchemaEvolution.html

## Storage and indexes

- A record store is one extent that can hold records of several types; all types share one primary key space, so two records of different types cannot share a primary key. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/Overview.html
- A primary key may be prefixed with a record type key, assigned a unique value per type, so records of one type are stored together. [S 2026-10-25] https://foundationdb.github.io/fdb-record-layer/FAQ.html
- Indexes may be defined on several record types, or on all (`addIndex(null, ...)`, universal index). [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/Overview.html
- Index states: disabled, write-only, readable; an index is built online in write-only state so concurrent updates are captured. [S 2026-10-25] https://foundationdb.github.io/fdb-record-layer/SchemaEvolution.html

## Evolution

- Each store header records the meta-data version last used; meta-data versions increase with every change; the version at which an index was added and last changed is tracked. [S 2026-10-25] https://foundationdb.github.io/fdb-record-layer/SchemaEvolution.html
- Persistent meta-data can be kept in an `FDBMetaDataStore`. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/Overview.html

## Types and records

- Records are protobuf messages; all top-level record types are fields of one union message (`RecordTypeUnion`, or a message with option `usage = UNION`), and each stored record is serialised as that union with one variant set. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/Overview.html
- The record is stored under its primary key, acting as a clustered index. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/Overview.html
- Index keys are key expressions: `field`, `nest`, `concat`, with `FanType.FanOut` (one entry per repeated value) or `FanType.Concatenate`. Index types: value, rank, version, text, count, sum. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/Overview.html
- Queries are built as `RecordQuery` objects (record type, filter, sort) and planned to a cursor; no text query language in the core API. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/Overview.html

## Serialization

- Records over about 100 KB are split across several key-values; record versions can be kept in separate keys; serialized records may be compressed or encrypted. [V 2026-10-25] https://foundationdb.github.io/fdb-record-layer/Overview.html

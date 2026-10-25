# KurrentDB (formerly EventStoreDB)

kind: system
home: https://docs.kurrent.io

## Data model

- Events live in streams, typically one stream per entity; a stream is an append-only sequence of state-change events. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/streams.html
- An event carries an id, an event type (free string), data, metadata and a content type. Stream metadata properties are `$`-prefixed; a stream's metadata lives in the `$$<stream>` metastream. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/streams.html
- `$all` holds every event in the database in log order; readable, not writable. Other system streams: `$settings`, `$stats`, `$scavenges`, `$persistentSubscriptionConfig`. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/streams.html
- Stream metadata: `$maxAge` (seconds), `$maxCount`, `$tb` (truncate before event number), `$cacheControl`, `$acl` with `$r`/`$w`/`$d`/`$mr`/`$mw`. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/streams.html
- Soft delete sets `$tb` to Int64 max; appending again reopens the stream. Hard delete appends a `$streamDeleted` tombstone and forbids the stream name forever. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/streams.html
- Link events are pointers to events in other streams; deleting the target leaves the link unresolved. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/streams.html
- v2 protocol `AppendRecord`: `record_id` (UUID, server-generated if absent), `properties` map (`$`-prefixed keys reserved for system, e.g. `$timestamp`, `$trace-id`), `SchemaInfo schema` {format, name, id}, `bytes data`. The schema name "identif[ies] the record type", e.g. "order-placed"; the record type is still a string, not a declared variant set. [V 2026-10-25] https://github.com/kurrent-io/KurrentDB/blob/master/proto/kurrentdb/protocol/v2/streams/streams.proto ; https://github.com/kurrent-io/KurrentDB/blob/master/proto/kurrentdb/protocol/v2/streams/shared.proto

## Storage

- Log is a sequence of chunk files; new data appended to the latest chunk; a chunk past 256 MiB is closed and a new one opened. Every write is flushed to disk by default. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/configuration/db-config.html
- Stream index: memtable of `MaxMemTableSize` (default 1000000) entries, flushed to PTables sorted by stream-name hash; two PTables at one level merge into the next level (level n holds about 2^(n-1) * 1M entries). Entry 24 bytes. `IndexCacheDepth` default 16 sets midpoint density. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/configuration/indexes.html
- `IndexEntry` is {Version int64, Stream uint64 hash, Position int64}, ordered by (Stream hash, Version, Position). The 64-bit hash is XXHash (high 32 bits) concatenated with Murmur3A (low 32 bits); collisions are possible and handled on read and in scavenge (`CollisionManagement`). [V 2026-10-25] https://github.com/kurrent-io/KurrentDB/blob/master/src/KurrentDB.Core/Index/IndexEntry.cs ; https://github.com/kurrent-io/KurrentDB/blob/master/src/KurrentDB.Core/Index/Hashes/CompositeHasher.cs ; https://github.com/kurrent-io/KurrentDB/blob/master/src/KurrentDB.Core/LogAbstraction/LogFormatAbstractor.cs
- Scavenge is manual (HTTP API or UI; an `AutoScavenge` plugin exists in source). It rewrites chunks without deleted/expired events (`$maxAge`, `$maxCount`, `$tb`, deleted streams) and may merge small chunks. Until scavenged, such events remain visible in `$all`. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/operations/scavenge.html
- Secondary indexes (25.1+): built-in category index and event-type index, stored in an embedded DuckDB separate from the log, read via `$all` with `StreamFilter.Prefix("$idx-ce-<cat>")` / `"$idx-et-<type>"`. Updated asynchronously after append: eventually consistent. Replace the link-event projections; docs claim up to 80% less storage. [V 2026-10-25] https://docs.kurrent.io/server/v26.0/features/indexes/secondary ; https://kurrentdb.kurrent.io/releases/kurrentdb/25-1/
- User-defined indexes (26.0): declared over HTTP with JS `filter` (e.g. `rec => rec.schema.name == 'OrderCreated'`) and `selector` (e.g. `rec => rec.value.country`, may return `skip`); at most 1 field, types string/double/int32/int64; read as `$idx-user-orders-by-country:Mauritius` or SQL in UI `select * from index:orders-by-country where field_country='Mauritius'`. Maintained per node by subscribing to the log. [V 2026-10-25] https://docs.kurrent.io/server/v26.0/features/indexes/user-defined

## Concurrency

- Optimistic concurrency per stream via expected revision: exact revision, or constants `NO_STREAM` (-1), `ANY` (-2, default), `EXISTS` (-4); mismatch fails with WrongExpectedVersion. [V 2026-10-25] https://github.com/kurrent-io/KurrentDB/blob/master/proto/kurrentdb/protocol/v2/streams/streams.proto
- No multi-version snapshot reads: the only isolation tool is the per-stream revision check at append. [U]
- Multi-stream atomic append (25.1+): `AppendSession` takes one `AppendRequest` per stream, each with its own expected revision; all-or-nothing; a stream may appear only once per session. [V 2026-10-25] https://github.com/kurrent-io/KurrentDB/blob/master/proto/kurrentdb/protocol/v2/streams/streams.proto ; https://kurrentdb.kurrent.io/releases/kurrentdb/25-1/
- `AppendRecords` decouples consistency checks from writes: a `ConsistencyCheck` (stream state check; query-predicate check is an UNIMPLEMENTED placeholder) may name a stream the request does not write, enabling Dynamic Consistency Boundary patterns. All failing checks are reported together. Records keep exact request order in the global log. [V 2026-10-25] https://github.com/kurrent-io/KurrentDB/blob/master/proto/kurrentdb/protocol/v2/streams/streams.proto

## Projections

- System projections (start Stopped): `$by_category` (splits stream id at separator, default `-`, position `first`; links into `$ce-<cat>`), `$by_event_type` (links into `$et-<type>`, not configurable), `$by_correlation_id` (`$bc-<id>`), `$stream_by_category` (`$category-<cat>`), `$streams`. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/projections/system.html
- User projections are JavaScript: selectors `fromAll`, `fromStream`, `fromStreams`, `fromCategory`, then `when({$init, $any, <EventType>: (state, event) => ...})`, `partitionBy`, `foreachStream`, `outputState` (writes `$projections-<name>-result`), `transformBy`, `filterBy`, `emit`, `linkTo`. Handlers dispatch on the event type string, a pattern match over an open set. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/projections/custom.html
- `fromAll` with named event-type handlers reads the per-event-type streams until caught up, then switches to reading the log. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/projections/custom.html
- Projections need JSON bodies, run only on the leader, and exclusively own the streams they emit to; applications may not append there. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/projections/

## Subscriptions

- Catch-up subscriptions: client keeps its position, ordered. Persistent subscriptions: server keeps position (checkpoints are `$SubscriptionCheckpoint` events in system streams), consumer groups, at-least-once, strategies RoundRobin (default), DispatchToSingle, Pinned (stream-hash, 1024 buckets), PinnedByCorrelation; no ordering guarantee; after `maxRetryCount` messages are parked. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/persistent-subscriptions.html

## Schema

- Core engine is schemaless: payload is opaque bytes plus a type string. [V 2026-10-25] https://docs.kurrent.io/server/v25.0/features/streams.html
- Schema registry (`src/SchemaRegistry`): named schemas with versions (`schema_version_id`, `version_number`), formats JSON, Protobuf, Avro, Bytes; compatibility modes BACKWARD, FORWARD, FULL, BACKWARD_ALL, FORWARD_ALL, FULL_ALL (against all prior versions), NONE. Only description and tags are mutable on a schema; a definition change is a new version. [V 2026-10-25] https://github.com/kurrent-io/KurrentDB/blob/master/proto/kurrentdb/protocol/v2/registry/shared.proto ; https://github.com/kurrent-io/KurrentDB/blob/master/proto/kurrentdb/protocol/v2/registry/schemas.proto
- Schema groups carry settings: auto-registration, validation, compatibility enforce+mode, data-format enforce, and a stream filter (regex or category) restricting which streams the group governs. [V 2026-10-25] https://github.com/kurrent-io/KurrentDB/blob/master/proto/kurrentdb/protocol/v2/registry/groups.proto
- Compatibility mode comments define BACKWARD as "new schemas can be used with data written by previous schemas" (adding a field ok, removing one not) and FORWARD as old readers reading new data. [V 2026-10-25] https://github.com/kurrent-io/KurrentDB/blob/master/proto/kurrentdb/protocol/v2/registry/shared.proto
- Events are never rewritten for evolution; old shapes stay in the log and readers upcast. Greg Young: a new version must be convertible from the old one, else it is a new event type. [S 2026-10-25] https://www.kurrent.io/blog/developers-tips-for-event-sourcing-eda/

# CockroachDB

kind: system
home: https://www.cockroachlabs.com/

## Concurrency

- Default SERIALIZABLE; READ COMMITTED opt-in; serializable transactions may restart and need client retries. [V 2026-10-25] https://docs.cockroachlabs.com/docs/stable/transactions

## Uniqueness

- REGIONAL BY ROW: indexes implicitly partitioned by `crdb_region`; unique checks look up every region in the same transaction; skipped for `gen_random_uuid()`, `UNIQUE (crdb_region, col)`, or region computed from key. [V 2026-10-25] https://www.cockroachlabs.com/blog/regional-by-row/

## Key encoding

- pkg/util/encoding: every key datum begins with a marker byte; 0x00 NULL ascending, 0xff NULL descending; bytesMarker 0x12, bytesDescMarker 0x13; ints use markers 0x80..0xfd (IntMin 0x80, intZero 136; uvarint values <= 109 stored in the marker byte itself, else marker IntMax-8+n then n big-endian bytes; MaxVarintLen 9). [V 2026-10-25] https://raw.githubusercontent.com/cockroachdb/cockroach/master/pkg/util/encoding/encoding.go
- Bytes ascending: 0x00 escaped as 0x00 0xff, terminator 0x00 0x01. Descending encodings complement every byte and use a separate marker so the terminator is known when decoding. [V 2026-10-25] same
- Arrays in keys: arrayKeyMarker then elements, terminator 0x00 (asc) or 0xff (desc); null inside an array is 0x01 (asc) or 0xfe (desc) so the terminator sorts below it. [V 2026-10-25] same
- JSON key encoding: one marker per JSON kind in a fixed order (empty array, null, string, number, false, true, array, object), each with a descending counterpart. [V 2026-10-25] same

## Replication

- Each range is a Raft group; the leaseholder alone serves strongly consistent reads and proposes writes, reading without a Raft round trip. Leader leases make the Raft leader the leaseholder, using store-wide failure detection instead of a liveness range; partition recovery under 20 seconds. [V 2026-10-25] https://docs.cockroachlabs.com/docs/stable/architecture/replication-layer

## Operations

- crdb_internal.transaction_contention_events: collection_ts, blocking_txn_id, blocking_txn_fingerprint_id, waiting_txn_id, waiting_txn_fingerprint_id, waiting_stmt_id, waiting_stmt_fingerprint_id, contention_duration, contending_key, contending_pretty_key, database/schema/table/index name, contention_type LOCK_WAIT or SERIALIZATION_CONFLICT. Insights record the conflicting transaction of a failed serializable execution. [V 2026-10-25] https://docs.cockroachlabs.com/docs/stable/monitor-and-analyze-transaction-contention
- Fingerprint ids are resolved after insert, sometimes much later; sql.contention.record_serialization_conflicts.enabled (default true) records 40001 errors as SERIALIZATION_CONFLICT events; one such event per transaction. [S 2026-10-25] https://github.com/cockroachdb/cockroach/issues/116569
- Admission control queues work for CPU and storage IO per node, ordered by priority (critical, regular, background) then transaction start time, favouring lock holders; covers SQL, bulk import, backup, schema change, replication, changefeeds. [V 2026-10-25] https://docs.cockroachlabs.com/docs/stable/admission-control
- Major upgrades are rolling, one node at a time, in a mixed-version cluster; finalization (automatic by default when all nodes run the new binary, disabled through cluster.auto_upgrade.enabled) enables features needing system-schema changes; rollback only before finalization. [V 2026-10-25] https://docs.cockroachlabs.com/docs/stable/upgrade-cockroach-version

## Design process

- The public docs/RFCS review process is deprecated; RFCs remain for archive; new process is internal. [V 2026-10-25] https://github.com/cockroachdb/cockroach/blob/master/docs/RFCS/README.md

## Retries over the wire

- All retry errors are SQLSTATE 40001 with "restart transaction"; kinds include RETRY_WRITE_TOO_OLD, RETRY_SERIALIZABLE, RETRY_ASYNC_WRITE_FAILURE, ReadWithinUncertaintyIntervalError. [V 2026-10-25] https://docs.cockroachlabs.com/docs/stable/transaction-retry-error-reference
- Server retries automatically when no results have reached the client, results stay under `sql.defaults.results_buffer.size` (16 KiB), and the transaction is implicit or single-statement; batching all statements of a transaction together raises the chance. [V 2026-10-25] same page
- Changefeeds: each row change delivered at least once; first emit per key ordered by timestamp; sinks Kafka, cloud storage, webhook, Pub/Sub; sinkless changefeeds stream over the SQL connection until it closes. [V 2026-10-25] https://docs.cockroachlabs.com/docs/stable/change-data-capture-overview

## Encryption at rest

- Store key (KEK, user-supplied) encrypts data keys (DEKs, generated); AES counter mode; every file of the store encrypted regardless of contents. Data keys rotate when the store key or cipher changes or after the rotation period; old files are not re-encrypted but rewritten by normal compaction churn. Backups are not encrypted by encryption at rest. [V 2026-10-25] https://docs.cockroachlabs.com/docs/stable/security-reference/encryption

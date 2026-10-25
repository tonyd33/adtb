# PostgreSQL

kind: system
home: https://www.postgresql.org/docs/

## Constraints

- PostgreSQL 18 adds temporal constraints: `WITHOUT OVERLAPS` on the last column (range or multirange) of a primary key or unique constraint, and `PERIOD` foreign keys; enforced through GiST indexes. [S 2026-10-25] https://www.postgresql.org/about/news/postgresql-18-released-3142/
- `EXCLUDE USING gist (room WITH =, during WITH &&)` exclusion constraints generalize uniqueness to any commutative operator with index support. [U] https://www.postgresql.org/docs/current/ddl-constraints.html
- CHECK constraints may not reference data other than the row being checked. [U] https://www.postgresql.org/docs/current/ddl-constraints.html


## Constraints

- Only UNIQUE, PRIMARY KEY, REFERENCES and EXCLUDE are deferrable; NOT NULL and CHECK are checked immediately on each row, not even at statement end. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-set-constraints.html

## Writes

- INSERT ON CONFLICT: only NOT DEFERRABLE constraints and unique indexes serve as arbiters; exclusion constraints not supported with DO UPDATE. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-insert.html
- ON CONFLICT DO UPDATE may not affect one row twice in a command (cardinality violation). [V 2026-10-25] https://www.postgresql.org/docs/current/sql-insert.html
- ON CONFLICT DO UPDATE on a partitioned table may not update the partition key so the row moves partition. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-insert.html
- RETURNING supports `old` and `new` qualifiers. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-insert.html
- MERGE: WHEN MATCHED / NOT MATCHED BY SOURCE / NOT MATCHED [BY TARGET], each with INSERT/UPDATE/DELETE/DO NOTHING; a target row joined to several source rows errors on the second change (unique or cardinality violation); docs point to INSERT ON CONFLICT as handling concurrent inserts. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-merge.html

## Concurrency

- SERIALIZABLE uses predicate locking (SSI) to detect serialization anomalies, including write skew. [S 2026-10-25] https://www.postgresql.org/docs/current/transaction-iso.html
- Application-level consistency checks under weaker levels need explicit locks (LOCK TABLE, SELECT FOR UPDATE) or a materialized conflict row. [S 2026-10-25] https://www.postgresql.org/docs/current/applevel-consistency.html
- A cross-partition UPDATE is delete plus insert; a concurrent updater or locker gets "tuple to be locked was already moved to another partition due to concurrent update", since the 6-byte TID cannot point into another partition. [S 2026-10-25] https://github.com/postgres/postgres/commit/f16241bef7cc271bff60e23de2f827a10e50dde8
- Unique index insertion fetches the heap tuple of an equal-key entry and waits if its inserter or deleter is in progress. [U] https://www.postgresql.org/docs/current/index-unique-checks.html

## Partitioning and inheritance

- Unique constraints on partitioned tables must include all partition key columns; each partition's index enforces uniqueness only within that partition. [V 2026-10-25] https://www.postgresql.org/docs/current/ddl-partitioning.html
- Updating a row's partition key moves it into a different partition. [V 2026-10-25] same page
- FKs referencing partitioned tables allowed since PG 12. [S 2026-10-25] https://www.postgresql.org/docs/release/12.0/
- Inheritance: indexes, unique constraints and FKs apply only to single tables, not children, on both referencing and referenced sides; "no good workaround". Children are separate physical storage; `ONLY` and `tableoid` expose the tag. [V 2026-10-25] https://www.postgresql.org/docs/current/ddl-inherit.html

## Indexes

- Partial unique index enforces uniqueness only among rows satisfying the predicate. Planner uses a partial index only when it proves the WHERE implies the predicate; parameterized clauses defeat that. [V 2026-10-25] https://www.postgresql.org/docs/current/indexes-partial.html
- An FK must reference columns of a non-partial unique index. [V 2026-10-25] https://www.postgresql.org/docs/current/ddl-constraints.html
- `_bt_check_unique` (nbtinsert.c) takes one `heapRel`, fetches with `SnapshotDirty`, returns `xwait` for in-progress conflicts. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/backend/access/nbtree/nbtinsert.c

## Global index proposals

- Herrera 2017-08-18: index tuple wrapper with relation OID. [V 2026-10-25] https://www.postgresql.org/message-id/20170818144047.tpfdfok7qijfopld%40alvherre.pgsql
- Ahmed and Akhtar 2019-10-30: widening CTID rejected as invasive; partition id as INCLUDE column preferred. [V 2026-10-25] https://www.postgresql.org/message-id/CALtqXTcurqy1PKXzP9XO=ofLLA5wBSo77BnUnYVEZpmcA3V0ag@mail.gmail.com
- HighGo (Huang, Zhang) 2022-11-17: storage per partition, insert probes every other partition's index; ATTACH re-validates. [V 2026-10-25] https://www.postgresql.org/message-id/184879c5306.12490ea581628934.7312528450011769010@highgo.ca
- Dilip Kumar 2025-06-06: 4-byte partition id per entry via catalog `pg_index_partitions`, not OID (detach/reattach and OID reuse ambiguity); detach invalidates mapping; inserts lock whole hierarchy; vacuum 15 s to 45 min on 1000 partitions until global index pass deferred (40 s). [V 2026-10-25] https://www.postgresql.org/message-id/CAFiTN-uyec_y2QS2whUam8Rp1M%2BPPGuP0Zz45uX_U6mhQ8mtRg%40mail.gmail.com
- Postgres Pro Enterprise ships `pgpro_gbtree`. [S 2026-10-25] https://postgrespro.com/docs/enterprise/current/pgpro-gbtree

## Storage

- Null columns take no data space; null bitmap one bit per column, present only if some column is null; tuple header 23 bytes plus MAXALIGN. [V 2026-10-25] https://www.postgresql.org/docs/current/storage-page-layout.html
- 1600 column limit; dropped columns still count. [V 2026-10-25] https://www.postgresql.org/docs/current/limits.html
- TOAST threshold about 2 kB row width. [V 2026-10-25] https://www.postgresql.org/docs/current/storage-toast.html
- `atthasmissing` fast defaults for attributes beyond a tuple's stored count. [U]

## Types and DDL

- `ALTER TYPE ... ADD VALUE` and `RENAME VALUE` for enums; no way to drop an enum value. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-altertype.html
- Dependency tracking refuses drops with dependents unless CASCADE; function bodies tracked only for `BEGIN ATOMIC`, not string literals. [V 2026-10-25] https://www.postgresql.org/docs/current/ddl-depend.html
- `SELECT *` in a view is expanded at creation. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-createview.html
- One field of a composite column is updated with `UPDATE mytab SET complex_col.r = (complex_col).r + 1`; no parentheses on the SET target, parentheses required on the right. [V 2026-10-25] https://www.postgresql.org/docs/current/rowtypes.html
- SET CONSTRAINTS switching a constraint from DEFERRED to IMMEDIATE checks outstanding modifications retroactively, during the SET CONSTRAINTS command. [U] https://www.postgresql.org/docs/current/sql-set-constraints.html

## Page layout

- PageHeaderData is 24 bytes: pd_lsn u64 at 0, pd_checksum u16 at 8, pd_flags u16 at 10, pd_lower u16 at 12, pd_upper u16 at 14, pd_special u16 at 16, pd_pagesize_version u16 at 18, pd_prune_xid u32 at 20. Line pointers (ItemIdData) are 4 bytes each: offset, length, flag bits. Default block size 8 kB, compile-time choice. [V 2026-10-25] https://www.postgresql.org/docs/current/storage-page-layout.html
- HeapTupleHeaderData is 23 bytes: t_xmin 4, t_xmax 4, t_cid/t_xvac 4, t_ctid 6, t_infomask2 2 (attribute count plus flags), t_infomask 2, t_hoff 1; null bitmap follows only when HEAP_HASNULL; t_hoff is a multiple of MAXALIGN. [V 2026-10-25] https://www.postgresql.org/docs/current/storage-page-layout.html
- Page checksum is 16 bits: an FNV-1a variant over the page as 32 parallel 32-bit sums, block number XORed in to catch misplaced pages, result (sum % 65535) + 1 so never zero; pd_checksum zeroed while computing. [V 2026-10-25] https://github.com/postgres/postgres/blob/master/src/include/storage/checksum_impl.h
- WAL records and full-page images in WAL are CRC-32C protected. [V 2026-10-25] https://www.postgresql.org/docs/current/wal-reliability.html
- full_page_writes: the first modification of a page after each checkpoint logs the whole page image, so a torn 8 kB write (16 sectors of 512 bytes) is repaired at recovery; can be turned off only on file systems that prevent partial page writes (ZFS). [V 2026-10-25] https://www.postgresql.org/docs/current/wal-reliability.html

## TOAST

- Varlena headers: 4-byte normal, 1-byte for values under 127 bytes, 18-byte TOAST pointer for out-of-line values; two bits of the length word reserved, so values cap at 1 GB. [V 2026-10-25] https://www.postgresql.org/docs/current/storage-toast.html
- TOAST starts when a row exceeds TOAST_TUPLE_THRESHOLD (about 2 kB) and compresses or moves fields until under TOAST_TUPLE_TARGET (about 2 kB); out-of-line values are chunks of at most TOAST_MAX_CHUNK_SIZE (about 2000 bytes) rows (chunk_id, chunk_seq, chunk_data) in a TOAST table; strategies PLAIN, EXTENDED, EXTERNAL, MAIN. [V 2026-10-25] https://www.postgresql.org/docs/current/storage-toast.html

## Planner and plan cache

- Partition pruning runs at plan time, at executor initialization (for parameters known then; reported as "Subplans Removed") and during execution (re-pruned whenever an execution parameter changes). Constraint exclusion uses CHECK constraints and is plan-time only. [V 2026-10-25] https://www.postgresql.org/docs/current/ddl-partitioning.html
- Planner handles up to a few thousand partitions if pruning leaves few; legacy inheritance up to perhaps a hundred children. [V 2026-10-25] same page
- Prepared statements: first five executions use custom plans; then a generic plan is used if its estimated cost is not much above the average custom cost; `plan_cache_mode` overrides. Replanned when used objects change definition, statistics are updated, or `search_path` changes. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-prepare.html
- pg_statistic per column: most_common_vals/freqs, histogram_bounds, n_distinct, null_frac, correlation; `default_statistics_target` 100. `pg_stats.inherited` separates whole-hierarchy statistics from one table's. Extended statistics kinds: dependencies, ndistinct, mcv. [V 2026-10-25] https://www.postgresql.org/docs/current/planner-stats.html

## nbtree (B-tree access method)

- nbtree implements Lehman and Yao (TODS 6(4), 1981): right-link to right sibling plus a high key per page; a descent that finds search key > high key follows the right-link, which detects concurrent splits without read locks across levels. Postgres still takes page-level read locks because buffers are shared. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/backend/access/nbtree/README
- Uniqueness of every B-tree key is achieved by treating heap TID as a tiebreaker attribute; logical duplicates sorted in heap TID order; L&Y need Ki < v <= Ki+1 with strictly unique keys. [V 2026-10-25] same README
- Internal-page entries and leaf high keys are "pivot tuples"; suffix truncation drops trailing key attributes not needed to separate the split halves, truncated attributes read as minus infinity; whole-attribute granularity only (Bayer and Unterauer, TODS 1977, simple prefix B-tree). Leaf split point chosen to make truncation effective while balancing space; split tries to keep duplicates on one page, falling back to a lopsided split. [V 2026-10-25] same README
- Leaf scans copy all matching TIDs of a leaf page to local memory, release the page lock, then visit the heap; scans stop "between" pages; items never move across pre-existing page boundaries. Scans remember the right-link as of reading the page. [V 2026-10-25] same README
- Locks: moving right or up may lock the next page before releasing the current; moving left or down never (deadlock). Ascent couples child and parent locks. [V 2026-10-25] same README
- VACUUM btbulkdelete scans the index in physical block order, takes a cleanup lock on every leaf page; a per-page vacuum cycle ID (btpo_cycleid) detects pages split since the cycle began, whose right-link to a lower block is followed so no tuple is missed. [V 2026-10-25] same README
- Page deletion: only empty leaf pages, two stages (unlink from parent and mark half-dead, then unlink from siblings); rightmost page of a level never deleted, so tree height never decreases; deleted pages recycled only after all snapshots older than deletion are gone (stamped with next XID). [V 2026-10-25] same README
- Index-only scans can never drop the leaf buffer pin since they consult the visibility map, not the heap, and could see a recycled TID. [V 2026-10-25] same README
- Simple deletion: a plain index scan that finds a heap tuple dead to all marks the index item LP_DEAD (with a share lock, like a hint bit); items deleted later under exclusive lock. LP_DEAD not set if page LSN changed since read. [V 2026-10-25] same README
- Bottom-up deletion (PG 14): triggered by an executor hint that an UPDATE produced a logically unchanged duplicate (HOT failed); deletes version-churn duplicates before a split; based on "most objects die young". [V 2026-10-25] same README; https://www.postgresql.org/docs/current/btree.html
- Rightmost-leaf fastpath: a backend caches the rightmost leaf it last inserted into and skips the descent for increasing keys. [V 2026-10-25] same README
- Items larger than 1/3 page are refused (page must hold a high key and two items). [V 2026-10-25] same README
- Deduplication (PG 13): lazily, only when a leaf page would otherwise split (after LP_DEAD removal and bottom-up deletion), equal non-pivot tuples merge into one posting list tuple: key once, then a sorted array of 6-byte heap TIDs. Unique indexes deduplicate only when an insert saw a duplicate in passing (version churn). Posting list split on overlapping insert swaps the new TID with the posting list's max TID in one atomic action. [V 2026-10-25] same README
- Deduplication is unusable for: nondeterministic collations, numeric (display scale), jsonb, float4/float8 (-0 vs 0), container types (composite, array, range), and INCLUDE indexes. [V 2026-10-25] https://www.postgresql.org/docs/current/btree.html
- BTPageOpaqueData (special area, 16 bytes): btpo_prev u32, btpo_next u32, btpo_level u32 (0 at leaf), btpo_flags u16, btpo_cycleid u16. Page level counts up from 0 at leaves. High key is the first item on non-rightmost pages. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/include/access/nbtree.h
- IndexTupleData: t_tid 6 bytes plus t_info u16 (bit15 has nulls, bit14 var-width, bit13 AM-defined, bits 12-0 size). Pivot tuple: t_tid | t_info | key values | [heap TID], key-column count in the low 12 bits of t_tid offset. Posting tuple: t_tid | t_info | key values | TID array, TID count in low 12 bits, BT_IS_POSTING 0x2000. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/include/access/nbtree.h https://raw.githubusercontent.com/postgres/postgres/master/src/include/access/itup.h
- BTMaxItemSize = MAXALIGN_DOWN((BLCKSZ - MAXALIGN(24 + 3*4) - MAXALIGN(16)) / 3) - MAXALIGN(6), 2704 bytes at 8 KiB pages (arithmetic from the macro). Fillfactor: leaf default 90, internal 70, single-value split 96. [V 2026-10-25] nbtree.h
- Visibility map: two bits per heap page, all-visible and all-frozen; set only by VACUUM, cleared by any modification of the page; index-only scans skip the heap fetch when all-visible is set. [V 2026-10-25] https://www.postgresql.org/docs/current/storage-vm.html
- Hash indexes store only the 4-byte hash code (lossy); no uniqueness, single column, `=` only; meta, bucket, overflow and bitmap pages; crash recoverable. [V 2026-10-25] https://www.postgresql.org/docs/current/hash-index.html

## MVCC internals

- HeapTupleHeaderData: t_xmin (4), t_xmax (4), t_cid/t_xvac union (4), t_ctid (6), t_infomask2 (2), t_infomask (2), t_hoff (1) = 23 bytes, then null bitmap. Cmin, Cmax and Xvac share one field; a tuple inserted and deleted in one transaction stores a backend-local combo command id. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/include/access/htup_details.h
- t_ctid is initialised to the tuple's own TID and set to the successor's TID on update; a tuple is latest iff xmax invalid or t_ctid points to itself. A chain follower must check the target's xmin equals the source's xmax, since vacuum may free the newer slot first and reuse it. [V 2026-10-25] same file
- Row moved to another partition: t_ctid set to the magic value block InvalidBlockNumber, offset MovedPartitionsOffsetNumber 0xfffd; SpecTokenOffsetNumber 0xfffe marks a speculative insertion token in t_ctid. ItemPointerData = 4-byte block + 2-byte offset. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/include/storage/itemptr.h
- Hint bits in t_infomask: HEAP_XMIN_COMMITTED 0x0100, HEAP_XMIN_INVALID 0x0200 (both = frozen), HEAP_XMAX_COMMITTED 0x0400, HEAP_XMAX_INVALID 0x0800, HEAP_XMAX_IS_MULTI 0x1000; lock bits KEYSHR 0x0010, EXCL 0x0040, LOCK_ONLY 0x0080. [V 2026-10-25] htup_details.h
- Line pointer ItemIdData 4 bytes: lp_off 15 bits, lp_flags 2 bits (UNUSED 0, NORMAL 1, REDIRECT 2, DEAD 3), lp_len 15 bits. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/include/storage/itemid.h
- SnapshotData MVCC fields: xmin (all XID < xmin visible), xmax (all XID >= xmax invisible), xip[] of in-progress XIDs with xmin <= xip < xmax, subxip[] with overflow flag, curcid. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/include/utils/snapshot.h
- Visibility code must check in-progress (PGPROC array or snapshot) before pg_xact, since commit is recorded in pg_xact before the proc's xid is cleared. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/backend/access/heap/heapam_visibility.c
- A committed hint bit may be set only if the commit record is flushed before the page, i.e. page LSN already newer than the commit LSN; otherwise deferred to a later look. Aborted hints always settable. Setting hints marks the buffer dirty (MarkBufferDirtyHint). [V 2026-10-25] heapam_visibility.c
- Hint bits: first reader after commit sets them; index scans fetching single tuples can write one page many times as hints accumulate. [V 2026-10-25] https://wiki.postgresql.org/wiki/Hint_Bits
- wal_log_hints writes a full page image on the first hint-bit-only change after a checkpoint; with data checksums enabled hint updates are always WAL-logged. [V 2026-10-25] https://www.postgresql.org/docs/current/runtime-config-wal.html
- pg_xact (clog): 2 bits per transaction, 4 per byte, BLCKSZ*4 per page (32768 per 8 KiB page). [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/backend/access/transam/clog.c
- FullTransactionId: 64 bits = 32-bit epoch << 32 | 32-bit xid, used in memory; tuples still store 32-bit xids. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/include/access/transam.h
- XIDs are 32-bit modulo 2^32: 2 billion older, 2 billion newer. Freezing sets a flag (9.4+) treating xmin as older than all; FrozenTransactionId 2, BootstrapTransactionId 1. autovacuum_freeze_max_age default 200 million; warnings at 40 million from wraparound, refusal to assign XIDs at 3 million. pg_xact about 50 MB at 200 million (500 MB at 2 billion); pg_commit_ts about 2 GB (20 GB). Multixacts have their own 32-bit wraparound. [V 2026-10-25] https://www.postgresql.org/docs/current/routine-vacuuming.html
- Visibility map: 2 bits per heap page, all-visible and all-frozen; set only by vacuum, cleared by any modification on the page. Index-only scans check the bit and fetch the heap only when it is clear; map is four orders of magnitude smaller than the heap. [V 2026-10-25] https://www.postgresql.org/docs/current/storage-vm.html https://www.postgresql.org/docs/current/indexes-index-only-scans.html

## HOT (README.HOT)

- An update that changes no column referenced by any non-summarizing index (key, expression, or partial-index predicate) and fits on the same page makes a heap-only tuple with no index entries; old version flagged HEAP_HOT_UPDATED, new HEAP_ONLY_TUPLE, linked by t_ctid. Index points to the chain root line pointer. HOT chains never cross pages. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/backend/access/heap/README.HOT
- Pruning turns a dead root into LP_REDIRECT to the next live member and frees intermediate heap-only tuples without touching indexes; a fully dead chain leaves an LP_DEAD stub until vacuum removes index entries. Needs a buffer cleanup lock; triggered on page access when pd_prune_xid set and free space < max(fillfactor target, BLCKSZ/10). Line pointers capped at MaxHeapTuplesPerPage. [V 2026-10-25] README.HOT
- HOT-safety compares old and new values bitwise, not by type equality. Summarizing indexes (BRIN) do not block HOT but still receive the new values. [V 2026-10-25] README.HOT
- CREATE INDEX over broken HOT chains points entries at the root TID with the live tuple's key; indcheckxmin hides the index from snapshots older than the build. CREATE INDEX CONCURRENTLY waits so every writer treats the new index as HOT-blocking first. [V 2026-10-25] README.HOT
- HOT forecloses recompute-the-key vacuum: the root's data may be gone. [V 2026-10-25] README.HOT

## SSI internals (README-SSI)

- SIREAD locks: tuple, page, relation; heap scan locks the relation; every visible tuple read is locked; finer locks promoted to coarser when memory runs short; lock table in fixed shared memory set at start. Insert conflicts only with relation-level heap locks; heap page locks only aggregate tuple locks. [V 2026-10-25] https://raw.githubusercontent.com/postgres/postgres/master/src/backend/storage/lmgr/README-SSI
- Index AMs lock the gaps a scan covered; B-tree locks leaf pages; an equality probe on a unique index finding a visible unmodified tuple needs no index lock. An update that moves a row into a scanned range must conflict; one that expires an entry and adds one for the same row need not. [V 2026-10-25] README-SSI
- A SIREAD lock on one tuple version does not cover later versions; README proves the extra edges would be false positives. [V 2026-10-25] README-SSI
- Table rewrite (CLUSTER, ALTER TABLE) promotes tuple and page SIREAD locks to relation locks; dropping an index replaces its gap locks with a relation lock on the heap. [V 2026-10-25] Ports, Grittner VLDB 2012 sect. 5.2
- Committed transactions beyond a fixed count are summarized: SIREAD locks reassigned to one dummy transaction, conflict-out kept as a 64-bit commit sequence number per xid in SLRU, so memory is bounded with more false positives. [V 2026-10-25] README-SSI; Ports, Grittner sect. 6.2
- PostgreSQL 17 replaced VACUUM's flat dead-TID array (1 GB cap, about 179M TIDs) with TidStore on an adaptive radix tree; up to 20x less memory, fewer index vacuum passes. [S 2026-10-25] https://pganalyze.com/blog/5mins-postgres-17-faster-vacuum-adaptive-radix-trees

## Buffer manager and durability

- Clock sweep: per-buffer usage count bumped on pin, capped at BM_MAX_USAGE_COUNT = 5; the hand decrements nonzero counts and takes the first unpinned zero-count buffer. In master (2026) the buffer state is a 64-bit word: 18 refcount bits, 4 usage bits, 12 flag bits, 20 lock bits. [V 2026-10-25] https://github.com/postgres/postgres/blob/master/src/backend/storage/buffer/README https://github.com/postgres/postgres/blob/master/src/include/storage/buf_internals.h
- Buffer rings: seq scans use a 256 KB ring, VACUUM a ring sized by vacuum_buffer_usage_limit, bulk writes (COPY IN, CTAS) 16 MB capped at 1/8 of shared_buffers. A dirtied ring buffer in a seq scan is dropped from the ring rather than forcing a WAL flush. [V 2026-10-25] same README
- bgwriter scans ahead of the clock hand writing dirty, unpinned, zero-usage buffers. [V 2026-10-25] same README
- History: 8.0.2 replaced ARC with 2Q to avoid a pending US patent on ARC; 8.1 switched to clock sweep. [V 2026-10-25] https://www.postgresql.org/docs/release/8.0.2/ https://www.postgresql.org/docs/release/8.1.0/
- shared_buffers default 128MB; guidance 25% of RAM, above 40% unlikely to help since Postgres also relies on the OS cache. [V 2026-10-25] https://www.postgresql.org/docs/current/runtime-config-resource.html
- io_method: worker, io_uring (needs liburing build), sync. debug_io_direct: data, wal, wal_init use O_DIRECT (F_NOCACHE on macOS, FILE_FLAG_NO_BUFFERING on Windows); default off. [V 2026-10-25] https://www.postgresql.org/docs/current/runtime-config-resource.html https://www.postgresql.org/docs/current/runtime-config-developer.html
- full_page_writes: whole page image logged on first modification after a checkpoint, against torn writes during OS crash. wal_compression (pglz, lz4, zstd) compresses those images. [V 2026-10-25] https://www.postgresql.org/docs/current/runtime-config-wal.html
- wal_sync_method: open_datasync, fdatasync, fsync, fsync_writethrough, open_sync. wal_init_zero zero-fills new WAL files; wal_recycle renames old ones. [V 2026-10-25] same page
- Group commit: commit_delay (default 0 microseconds) waits before a WAL flush only if commit_siblings (default 5) transactions are active; since 9.3 the first flusher waits, followers wait only until the leader's flush completes. [V 2026-10-25] same page
- Checkpoint: flush every dirty page, write a checkpoint record; recovery starts at its redo record; earlier WAL segments recycled. checkpoint_timeout 5 min, max_wal_size 1 GB, checkpoint_completion_target 0.9 spreads the writes; checkpoint_flush_after 256kB on Linux. [V 2026-10-25] https://www.postgresql.org/docs/current/wal-configuration.html https://www.postgresql.org/docs/current/runtime-config-wal.html
- fsyncgate 2018: Linux marked pages clean after a writeback error and reported it only to descriptors open at the time; retrying fsync could falsely succeed. Postgres now PANICs on fsync failure and replays WAL, backpatched to 9.4. [V 2026-10-25] https://wiki.postgresql.org/wiki/Fsync_Errors
- old_snapshot_threshold (vacuum may remove rows a running snapshot could see, raising "snapshot too old" on access) added in 9.6, removed in 17; may return with a better implementation. [S 2026-10-25] https://www.postgresql.org/docs/release/17.0/

## Replication and backup

- Serializable is not available in hot standby; setting it raises an error. Standby connections are strictly read-only and never get a transaction id. DDL, GRANT/REVOKE, ANALYZE/VACUUM/CLUSTER/REINDEX are refused during recovery. [V 2026-10-25] https://www.postgresql.org/docs/current/hot-standby.html
- Recovery conflicts: replayed AccessExclusive locks (explicit LOCK and DDL) conflict with standby table access; replayed vacuum cleanup conflicts with standby snapshots that can see removed rows. Conflicting queries are cancelled after max_standby_streaming_delay (archive: max_standby_archive_delay). hot_standby_feedback stops the primary's VACUUM removing rows a standby still sees, at the cost of primary bloat. [V 2026-10-25] same page
- synchronous_commit levels: off (no wait; loss bounded by 3x wal_writer_delay, no inconsistency), local (local flush only), remote_write (standby wrote to its file system, not durable against standby OS crash), on (standby flushed to durable storage), remote_apply (standby applied and visible to its queries; much larger commit delay). [V 2026-10-25] https://www.postgresql.org/docs/current/runtime-config-wal.html
- Group commit knobs: commit_delay waits before a WAL flush only if at least commit_siblings (default 5) other transactions are active. [V 2026-10-25] same page
- Logical replication does not replicate schema or DDL; schema changes are synced by hand, and data that does not fit the subscriber's schema stops replication until fixed; additive changes should be applied to the subscriber first. Sequences, large objects, views not replicated. [V 2026-10-25] https://www.postgresql.org/docs/current/logical-replication-restrictions.html
- Logical slots emit each change once in normal operation, but slot position is persisted only at checkpoint, so after a crash changes are resent; consumers must be idempotent. Slots retain WAL and catalog rows even with no consumer. Slot creation exports a snapshot showing exactly the state after which changes are streamed. Failover slots: `failover` option, sync_replication_slots on the standby, synchronized_standby_slots on the primary. [V 2026-10-25] https://www.postgresql.org/docs/current/logicaldecoding-explanation.html
- PITR = file-system base backup plus continuous WAL archive (archive_command / archive_library); recovery_target_{time,xid,lsn,name,immediate}; finishing PITR starts a new timeline with a .history file so old WAL is not overwritten. [V 2026-10-25] https://www.postgresql.org/docs/current/continuous-archiving.html
- PG 17 incremental backup: pg_basebackup --incremental=<previous manifest>, needs WAL summaries (summarize_wal) covering the LSNs since the prior backup; restored with pg_combinebackup over the whole chain. [V 2026-10-25] same page
- pg_verifybackup checks the manifest, file presence and checksums, and parses the needed WAL with pg_waldump; it states it cannot catch every problem recovery would, e.g. WAL with correct checksums but nonsensical actions. [V 2026-10-25] https://www.postgresql.org/docs/current/app-pgverifybackup.html
- Thomas Munro's patch "SERIALIZABLE on standby servers" (SERIALIZABLE READ ONLY DEFERRABLE on hot standby via safe-snapshot markers in WAL) ran 2016 to 2019 and was withdrawn 2019-07. [V 2026-10-25] https://commitfest.postgresql.org/22/1799/
- Grittner 2011 proposal: WAL carries "this snapshot is being tested" and "the snapshot from the last test is good" markers; the standby takes a snapshot and waits until it is known safe. [V 2026-10-25] https://www.postgresql.org/message-id/4D3735E30200002500039869@gw.wicourts.gov

## Operations: plan inspection

- EXPLAIN options (PG 18) with defaults: ANALYZE false, VERBOSE false, COSTS true, SETTINGS false, GENERIC_PLAN false, BUFFERS false, SERIALIZE none, WAL false, TIMING true, SUMMARY false (true with ANALYZE), MEMORY false, FORMAT text. ANALYZE executes the statement and its side effects happen; the doc advises BEGIN; EXPLAIN ANALYZE ...; ROLLBACK. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-explain.html
- auto_explain logs plans of statements over log_min_duration (default -1, off); log_analyze (off) logs EXPLAIN ANALYZE output at a per-node timing cost; log_buffers, log_nested_statements (off), sample_rate (1). Superuser-only settings. [V 2026-10-25] https://www.postgresql.org/docs/current/auto-explain.html

## Operations: statistics views

- pg_stat_statements: queryid from the post-parse-analysis tree; constants replaced by $n; queryid not stable across major versions or platforms; columns calls, total/min/max/mean_exec_time, rows, plans, plan times, shared/local/temp block counts, WAL and JIT counts. pg_stat_statements.max default 5000, least-executed entries deallocated, counted in pg_stat_statements_info.dealloc. Needs shared_preload_libraries, so adding it needs a restart. [V 2026-10-25] https://www.postgresql.org/docs/current/pgstatstatements.html
- wait_event_type values: Activity, BufferPin, Client, Extension, InjectionPoint, IO, IPC, Lock (heavyweight), LWLock, Timeout. [V 2026-10-25] https://www.postgresql.org/docs/current/monitoring-stats.html
- Cumulative statistics live in shared memory, written to pg_stat/ on clean shutdown and kept across restart; reset after crash, base backup or PITR. Accessed values are cached until transaction end by default (stats_fetch_consistency: none, cache, snapshot). [V 2026-10-25] https://www.postgresql.org/docs/current/monitoring-stats.html
- Predicate locks show in pg_locks with mode SIReadLock; tuple locks may be promoted to page and relation locks when the predicate lock table runs short, raising the serialization-failure rate; max_pred_locks_per_transaction/relation/page control it. Error text: "could not serialize access due to read/write dependencies among transactions". [V 2026-10-25] https://www.postgresql.org/docs/current/transaction-iso.html
- Serialization failures carry a DETAIL "Reason code: ..." such as "Canceled on identification as a pivot, during commit attempt"; they do not name the conflicting transaction. [S 2026-10-25] https://pganalyze.com/docs/log-insights/app-errors/U139

## Operations: integrity checking

- amcheck: bt_index_check(index, heapallindexed, checkunique) under AccessShareLock; bt_index_parent_check(index, heapallindexed, rootdescend, checkunique) under ShareLock, adds parent/child and downlink checks; gin_index_check; verify_heapam(rel, on_error_stop, check_toast, skip, startblock, endblock) returns one row per corruption (blkno, offnum, attnum, msg) while B-tree checks raise errors. No repair: "There is no general method of repairing problems that amcheck detects." [V 2026-10-25] https://www.postgresql.org/docs/current/amcheck.html

## Operations: resource limits and configuration

- statement_timeout, lock_timeout (per acquisition), idle_in_transaction_session_timeout, idle_session_timeout, transaction_timeout: all default 0 (off). statement and lock timeouts abort the statement; the idle and transaction timeouts terminate the session. Prepared transactions are exempt from transaction_timeout. [V 2026-10-25] https://www.postgresql.org/docs/current/runtime-config-client.html
- transaction_timeout was added in PostgreSQL 17. [V 2026-10-25] https://www.postgresql.org/docs/17/release-17.html
- pg_cancel_backend(pid) cancels the current query; pg_terminate_backend(pid, timeout) ends the session, waiting up to timeout ms. Allowed to superusers, members of the target role, and pg_signal_backend. [V 2026-10-25] https://www.postgresql.org/docs/current/functions-admin.html
- pg_settings.context: internal (fixed at build or initdb), postmaster (restart), sighup (reload), superuser-backend and backend (at connection start), superuser and user (SET in session). [V 2026-10-25] https://www.postgresql.org/docs/current/view-pg-settings.html

## Operations: protocol

- Startup message carries the protocol version; a major mismatch is refused; a minor mismatch may be answered with NegotiateProtocolVersion naming the highest supported minor. 3.0 since 7.4; 3.2 in PG 18 (longer cancel key); 3.1 skipped over a pgbouncer bug. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-overview.html

## Derived data

- `REFRESH MATERIALIZED VIEW` completely replaces contents by rerunning the query; `CONCURRENTLY` needs a populated view with a unique index on plain columns covering all rows, does not block readers, one refresh at a time. Not incremental. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-refreshmaterializedview.html
- Generated columns: stored (computed on write) or virtual (computed on read; default since PG 18). Expressions use only immutable functions, reference only the current row, no subqueries, no other generated column; virtual ones only built-in types and functions. [V 2026-10-25] https://www.postgresql.org/docs/current/ddl-generated-columns.html https://www.postgresql.org/docs/release/18.0/
- PG 18 temporal constraints: `WITHOUT OVERLAPS` on PK/UNIQUE, `PERIOD` on FKs, on the last column. [V 2026-10-25] https://www.postgresql.org/docs/release/18.0/ (upgrades the [S] bullet above)
- Rule system is query rewrite before planning; views are implemented as SELECT rules. [V 2026-10-25] https://www.postgresql.org/docs/current/rules.html
- Triggers: BEFORE/AFTER/INSTEAD OF (views, row only), row or statement level; AFTER triggers may request transition tables of changed rows; same-event triggers fire alphabetically by name; cascading triggers have no depth limit and avoiding infinite recursion is the programmer's job. [V 2026-10-25] https://www.postgresql.org/docs/current/trigger-definition.html

## COPY

- Formats: text (default), csv, binary. COPY FROM invokes triggers and checks constraints on the destination table; it does not invoke rules. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-copy.html
- FREEZE loads rows already frozen; only if the table was created or truncated in the current subtransaction, no cursors open, no older snapshots held; not for partitioned or foreign tables. [V 2026-10-25] same page
- ON_ERROR stop|ignore (added in 17); ignore discards rows whose column values fail type input, text/csv only; REJECT_LIMIT caps tolerated conversion errors; LOG_VERBOSITY reports skipped rows. A constraint failure still fails the whole COPY. [V 2026-10-25] same page; 17 release https://www.postgresql.org/docs/release/17.0/ [S 2026-10-25] for the constraint-failure point

## pg_dump

- Formats plain, custom (-Fc), directory (-Fd, parallel with -j via synchronized snapshots), tar. [V 2026-10-25] https://www.postgresql.org/docs/current/app-pgdump.html
- Sections: pre-data (definitions), data (rows, sequence values, statistics), post-data (indexes, triggers, rules, constraints other than validated check/not-null). --serializable-deferrable for a snapshot consistent with later states. Data-only restore with FKs uses --disable-triggers. [V 2026-10-25] same page

## Extensions

- An extension is a control file, versioned script files ext--1.0.sql and update scripts ext--1.0--1.1.sql, optional shared library; ALTER EXTENSION UPDATE chains update scripts. pg_dump emits CREATE EXTENSION rather than member objects. trusted extensions installable by non-superusers, flagged as security-exposed. [V 2026-10-25] https://www.postgresql.org/docs/current/extend-extensions.html
- User-defined aggregates: sfunc, stype, initcond, finalfunc; combinefunc for parallel partial aggregation, serialfunc/deserialfunc for internal state; moving-aggregate msfunc/minvfunc; ordered-set aggregates with direct arguments. [V 2026-10-25] https://www.postgresql.org/docs/current/xaggr.html

## Index access methods

- Index AM interface: a handler returns IndexAmRoutine of callbacks (ambuild, aminsert, ambeginscan, amgettuple, amgetbitmap, amcostestimate, amvacuumcleanup...); the core knows nothing of an index beyond it; indexes map keys to TIDs on standard pages via the buffer manager. [V 2026-10-25] https://www.postgresql.org/docs/current/indexam.html
- GiST support methods: consistent, union, compress, decompress, penalty, picksplit, same; optional distance, fetch, options, sortsupport, translate_cmptype. "The GiST layer itself takes care of concurrency, logging and searching the tree structure." [V 2026-10-25] https://www.postgresql.org/docs/current/gist.html

## Inheritance and foreign tables

- Table inheritance: check and not-null constraints inherited; unique, primary key and FK constraints apply only to single tables, on both referencing and referenced sides. [V 2026-10-25] https://www.postgresql.org/docs/current/ddl-inherit.html
- postgres_fdw: IMPORT FOREIGN SCHEMA creates foreign table definitions; WHERE, joins on one server, and UPDATE/DELETE pushed down; check constraints not imported automatically since they may evaluate differently remotely. [V 2026-10-25] https://www.postgresql.org/docs/current/postgres-fdw.html

## Wire protocol

- Protocol versions: 3.2 from PostgreSQL 18, 3.1 reserved and never used (skipped due to a pgbouncer bug), 3.0 from 7.4, 2.0 up to 13. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-overview.html
- 3.2 enlarged the cancel secret key from 4 bytes to variable length; BackendKeyData and CancelRequest changed accordingly. PG 18 makes cancel keys 256 bits, only when both sides speak 3.2. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-overview.html, https://www.postgresql.org/docs/18/release-18.html
- A server asked for an unsupported minor version may answer NegotiateProtocolVersion with the highest minor it supports; the client decides whether to continue. libpq 18 adds `min_protocol_version` and `max_protocol_version`. [V 2026-10-25] same pages
- Message: one type byte, an int32 length counting itself but not the type byte, then the body; the startup message has no type byte. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-overview.html
- Extended query: Parse (text to prepared statement), Bind (statement plus parameters to portal), Execute. Prepared statements and portals exist only within a session, never shared across sessions. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-overview.html
- Execute carries a max row count (0 = all); stopping early sends PortalSuspended and the client issues another Execute on the portal. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-flow.html
- Pipelining: the extended protocol allows sending a series of queries without waiting; after an error the backend discards messages until Sync, so later pipelined commands are skipped without BEGIN/COMMIT. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-flow.html
- Cancellation: the client opens a new connection and sends CancelRequest with the PID and secret key from startup. libpq 17 adds non-blocking and encrypted cancel requests. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-flow.html, https://www.postgresql.org/docs/17/release-17.html
- NoticeResponse may arrive from outside activity (e.g. fast shutdown). NotificationResponse (LISTEN/NOTIFY) is sent only outside a transaction, possibly just before ReadyForQuery. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-flow.html
- TLS: SSLRequest answered by one byte S or N; a client must read exactly one byte before TLS (CVE-2021-23222 buffer stuffing). Direct TLS without SSLRequest (PG 17, `sslnegotiation=direct`) requires ALPN, protocol id `postgresql` in the IANA registry. GSSENCRequest similarly. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-flow.html, https://www.postgresql.org/docs/17/release-17.html
- COPY: CopyInResponse/CopyOutResponse then CopyData messages (one per row out) and CopyDone or CopyFail; copy-both used by walsender START_REPLICATION. [V 2026-10-25] https://www.postgresql.org/docs/current/protocol-flow.html
- libpq 17 adds `PQsendPipelineSync()` (sync without flush). [V 2026-10-25] https://www.postgresql.org/docs/17/release-17.html

## Authentication

- SCRAM-SHA-256 and SCRAM-SHA-256-PLUS (channel binding type `tls-server-end-point`). Flow: AuthenticationSASL (mechanism list), SASLInitialResponse (client-first), AuthenticationSASLContinue, SASLResponse (client-final), AuthenticationSASLFinal then AuthenticationOk. [V 2026-10-25] https://www.postgresql.org/docs/current/sasl-authentication.html
- PG 18: `oauth` method with validator libraries (`oauth_validator_libraries`); MD5 passwords deprecated; SCRAM passthrough to postgres_fdw and dblink (`use_scram_passthrough`, `scram_client_key`, `scram_server_key`). [V 2026-10-25] https://www.postgresql.org/docs/18/release-18.html

## Notifications and change feeds

- NOTIFY payload max 8000 bytes by default; queue 8GB; when full, NOTIFY transactions fail at commit; delivered only at commit, discarded on abort; identical channel+payload folded within one transaction; same-transaction order kept, cross-transaction in commit order; a NOTIFY transaction cannot be prepared for 2PC. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-notify.html
- Logical decoding: WAL decoded by an output plugin into a change stream per replication slot, in the order made; each change once per slot in normal operation but resent after a crash since slot position persists only at checkpoint; one consumer per slot; slots retain WAL and can fill disk; slot creation exports a snapshot. [V 2026-10-25] https://www.postgresql.org/docs/current/logicaldecoding-explanation.html

## Scalar types

- numeric(p, s): excess fractional digits are rounded (ties away from zero); excess integer digits raise an error; max declared precision 1000; NaN, Infinity, -Infinity allowed only in unconstrained numeric. [V 2026-10-25] https://www.postgresql.org/docs/current/datatype-numeric.html
- For both numeric and float, NaN values are treated as equal to each other and greater than all non-NaN values, so they sort and index. [V 2026-10-25] same
- Integer types raise an error on out-of-range values. serial columns may have gaps: a value taken from the sequence is used up even when the inserting transaction rolls back. [V 2026-10-25] same
- Order of evaluation of subexpressions is not defined; WHERE x > 0 AND y/x > 1.5 is unsafe; CASE fixes order except that IMMUTABLE constant subexpressions may be folded at plan time (CASE WHEN x > 0 THEN x ELSE 1/0 END fails) and aggregates inside CASE are computed over all rows first. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-expressions.html sec. 4.2.14
- Sequences: nextval and setval are never rolled back, so they cannot give gapless numbering; with CACHE > 1 values are distinct but may be generated out of order across sessions. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-createsequence.html
- Collations: deterministic means equal only when byte-identical; nondeterministic (case- or accent-insensitive, normal-form-insensitive) collations block B-tree deduplication and some pattern matching. Builtin pg_c_utf8 and pg_unicode_fast sort by code point, stable within a major version. [V 2026-10-25] https://www.postgresql.org/docs/current/collation.html
- Collation provider version is recorded per collation; a mismatch warns, since changed collation definitions can corrupt indexes; affected objects must be REINDEXed, then ALTER COLLATION ... REFRESH VERSION, which does not check the rebuild. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-altercollation.html
- timestamptz is stored as UTC and the input zone is not retained; resolution 1 microsecond; interval is stored as months, days, microseconds, kept separate because month lengths vary and a day can be 23 or 25 hours under DST. [V 2026-10-25] https://www.postgresql.org/docs/current/datatype-datetime.html
- uuidv7() (PG 18): millisecond Unix timestamp plus sub-millisecond timestamp plus random; optional shift interval. [V 2026-10-25] https://www.postgresql.org/docs/current/functions-uuid.html

## Privileges and row security

- With RLS enabled and no policy, a default-deny policy applies: no rows visible or modifiable. Permissive policies combine with OR, restrictive with AND. USING filters visible rows; WITH CHECK constrains new rows; USING alone implies an identical WITH CHECK. [V 2026-10-25] https://www.postgresql.org/docs/current/ddl-rowsecurity.html
- Superusers and BYPASSRLS roles always bypass RLS; table owners bypass unless FORCE ROW LEVEL SECURITY. [V 2026-10-25] same
- "Referential integrity checks, such as unique or primary key constraints and foreign key references, always bypass row security"; the docs warn of "covert channel" leaks through them. [V 2026-10-25] same
- security_barrier views stop user-chosen functions seeing rows before the view's filter; may be slower since "the fastest possible plan must be rejected". LEAKPROOF functions (no side effects, no argument-dependent errors) may be pushed below the barrier; a function that errors on overflow or division by zero is not leakproof. [V 2026-10-25] https://www.postgresql.org/docs/current/rules-privileges.html
- Privileges: SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER, CREATE, CONNECT, TEMPORARY, EXECUTE, USAGE, SET, ALTER SYSTEM, MAINTAIN. PUBLIC gets EXECUTE on new functions and CONNECT, TEMPORARY on databases by default; column-level GRANT exists; owners hold all privileges implicitly; revoking a grant option cascades. [V 2026-10-25] https://www.postgresql.org/docs/current/ddl-priv.html
- SECURITY INVOKER (default) runs a function with the caller's privileges, SECURITY DEFINER with the owner's; definer functions must pin search_path (pg_temp last) or a temporary table can shadow the intended one. [V 2026-10-25] https://www.postgresql.org/docs/current/sql-createfunction.html
- Upstream TDE unmerged after patches in 2016, 2018, 2019; proposed design is cluster-wide AES with a key hierarchy; shared buffers stay plaintext; LSN-based nonces are reused when hint bits change pages within a checkpoint, so wal_log_hints would be required; whether checksums are over ciphertext is open. [V 2026-10-25] https://wiki.postgresql.org/wiki/Transparent_Data_Encryption
- Generic WAL records let extension index AMs WAL-log page changes without a custom resource manager: GenericXLogStart, GenericXLogRegisterBuffer (returns a page copy to modify), GenericXLogFinish; assumes standard page layout; ignored by logical decoding. [V 2026-10-25] https://www.postgresql.org/docs/current/generic-wal.html
- An index AM without ampredlocks makes every serializable scan take a predicate lock on the whole index, conflicting with any concurrent insert into it. [V 2026-10-25] https://www.postgresql.org/docs/current/index-locking.html

---
accept:
  btree/build/merge-runs: {title: Sort each heap's entries separately and merge the runs into a bottom-up build, reason: merge sorted runs from each constructor heap}
  btree/cleanup/bottom-up: {title: 'Before a split, check duplicates'' rows and delete dead ones', reason: readers mark dead entries; splits first try deleting them}
  btree/cleanup/kill-bit: {title: A scan that finds a row dead to all marks its entry dead, reason: readers mark dead entries; splits first try deleting them}
  btree/ctor-rewrite/rewrite: {title: The operator moves rows to the new heaps and rewrites every entry, reason: a split or merge rewrites entries; one heap per constructor stays literal}
  btree/ctor-width/uvarint: {title: 'Order-preserving varint, one byte for small ids', reason: 'constructor ids are never reused, so a fixed width is a lifetime limit'}
  btree/dead-set/radix: {title: 'Radix tree keyed by constructor and block, offsets as a bitmap', reason: compact dead-TID set for one index pass per sum}
  btree/entry-order/after-key: {title: 'Sort by key, then constructor, then tid', reason: 'a TID names a row only within its heap, so the constructor id sorts right after the key'}
  btree/hash/eq-unique: {title: 'Hash ctor-set index for equality probes, entries hold hash, ctor and tid', reason: B+-trees serve every index; hashing only where equality is all that is asked}
  btree/ios/vm-per-heap: {title: 'Each constructor heap has a visibility map, chosen by the entry''s constructor', reason: same as mvcc/index-only}
  btree/keyenc/canon/canonicalize: {title: Encoding maps each equality class to one byte string, reason: equal values encode to equal bytes}
  btree/keyenc/memcmp: {title: Order-preserving byte strings compared with memcmp everywhere, reason: 'order-preserving byte keys for every key type, including sums'}
  btree/latch/olc: {title: Optimistic lock coupling with a version word per page, reason: optimistic lock coupling for read-mostly descents}
  btree/latch/racy/atomic-reads: {title: 'Page reads go through relaxed atomic or volatile loads, validated before use', reason: 'optimistic reads of pages use atomic loads, defined behaviour in Rust'}
  btree/merge/empty-only: {title: 'Only empty pages are removed, never merged', reason: 'merge only empty pages, as Postgres'}
  btree/posting/posting-pairs: {title: 'Key once, then a sorted array of constructor and tid pairs', reason: lazy deduplication of equal keys}
  btree/structure/blink: {title: Disk B+-tree with right-links and high keys, reason: 'B-link trees: proven on disk, concurrent, page-based'}
  btree/sum-field/by-tag: {title: Index only the constructor tag, reason: 'index a sum field whole, by tag, or by one constructor field'}
  btree/sum-field/ctor-field: {title: 'Index a payload field of one constructor, entries only for rows of it', reason: 'index a sum field whole, by tag, or by one constructor field'}
  btree/sum-field/implied/arm-scoped: {title: An arm whose pattern names the constructor may use the index, reason: an index on a constructor field is used only where the constructor is known}
  btree/sum-field/whole: {title: Index the whole sum value, reason: 'index a sum field whole, by tag, or by one constructor field'}
  btree/sum-key/tag-payload: {title: 'Constructor tag, then that constructor''s fields in order', reason: a sum key encodes as tag then payload}
  btree/sum-tag/order-key: {title: 'A per-constructor sort key in the catalog, densely insertable', reason: 'index order follows a sum declared order, so ordered scans use the index'}
  btree/varlen/escape-term: {title: Escape 0x00 inside the value and end it with a terminator, reason: escaped terminators keep variable-length keys order-preserving}
  buffer/async/uring-fallback: {title: io_uring on Linux behind an interface with portable backends, reason: 'io_uring where available, a blocking pool otherwise'}
  buffer/cache/vm-reserve/global-pid: {title: 'One page-id space over all storage, files are extents of it', reason: one page-id space across the single file}
  buffer/cache/vmcache: {title: Own buffer pool over anonymous virtual memory indexed by page id, reason: one page-id space over all heaps and indexes; many index entries may point at one heap page}
  buffer/checkpoint/fuzzy: {title: 'Log the dirty page table, redo from its oldest recLSN, flush continuously', reason: checkpoints do not stop writers}
  buffer/checkpoint/log-pin/flush-oldest: {title: 'When log space nears a limit, flush the frames with the oldest recLSN', reason: a checkpoint flushes the oldest dirty pages so the log can be truncated}
  buffer/faults/corrupt-detect: {title: 'Also bit rot, latent sector errors and misdirected writes, detected and halted on', reason: the spec promises crash safety and detection of corrupted pages}
  buffer/faults/stale-page/lsn-check: {title: Redo and reads compare the page LSN with the LSN the log or referrer expects, reason: a page older than the log says it should be is detected by its LSN}
  buffer/force/fdatasync: {title: 'Write then fdatasync on preallocated, zero-filled log segments', reason: fdatasync on the log}
  buffer/fsync-fail/panic-replay: {title: Crash and recover from the log, reason: after an fsync failure the only sound recovery is crash and replay from the log}
  buffer/group-commit/leader: {title: The first committer flushes for everyone queued behind it, reason: one committer flushes the log for every waiting transaction}
  buffer/io-path/direct: {title: 'O_DIRECT for data and log, the pool is the only cache', reason: 'the engine owns caching, and fsync recovery is sound only without the page cache'}
  buffer/io-path/log-align/slot-pad: {title: 'Each commit batch takes whole blocks, padding the rest', reason: log writes padded to aligned slots for direct I/O}
  buffer/memory/split: {title: A fixed page pool plus per-query arenas with per-operator limits, reason: a fixed page pool plus bounded per-query memory}
  buffer/platform/linux-first: {title: 'Linux tuned, other systems supported with slower paths', reason: Linux is the target; the I/O layer stays portable for embedding}
  buffer/test/crash-replay: {title: Record the block-level write stream and recover from every crash prefix, reason: deterministic simulation with injected storage faults and crash replay}
  buffer/test/dst: {title: Deterministic simulation of the whole engine with injected storage faults, reason: deterministic simulation with injected storage faults and crash replay}
  buffer/test/dst-threads/sim-scheduler: {title: A deterministic scheduler replaces the async runtime and OS threads in tests, reason: 'SSI is multi-threaded, so the simulator schedules threads'}
  buffer/test/fault-fs: {title: A fault-injecting file system under the real engine, reason: deterministic simulation with injected storage faults and crash replay}
  constraint/admit/bounded: {title: Refuse a constraint without an index-bounded incremental check, reason: only constraints with an index-bounded incremental check are admitted; the rest are refused at schema time}
  constraint/admit/reference-index/implied: {title: Each reference field has an implied index on the referring field over the referring constructor set, reason: a reference's target check and delete actions probe an index like every other constraint; the implied index costs nothing where a declared one already leads with the field}
  constraint/admit/transition-negation/monotone-only: {title: Refuse inserted or deleted under negation or inside an aggregate, reason: 'a change touching nothing a constraint reads must leave it empty, so skipping untouched constraints stays correct'}
  constraint/aggregates/maintained: {title: Admit aggregate constraints whose aggregate the engine maintains per group, reason: a group aggregate maintained from each change set is index-bounded}
  constraint/assertion: {title: Arbitrary database-wide assertions over any query, reason: 'one general form, the assertion, with the other kinds as its common cases'}
  constraint/at-least-one: {title: 'Declarable inclusion from parent into a relationship, checked at commit', reason: same disjunctive inclusion as totality; leaving it out makes users re-emulate what the project removes}
  constraint/cardinality: {title: Bounds on how many related rows each row has, reason: 'one general form, the assertion, with the other kinds as its common cases'}
  constraint/concurrent/serial-checks: {title: Constraint checks' reads are validated serializably at any level, reason: under serializable isolation concurrent checks cannot jointly violate}
  constraint/domain: {title: 'Predicates on a field''s type, independent of the row', reason: 'one general form, the assertion, with the other kinds as its common cases'}
  constraint/enforce/delta: {title: Compile each constraint into per-relation delta checks, reason: checks run on the change set}
  constraint/evolve/carry: {title: Each structural operator rewrites the constraints it touches, reason: operators carry constraints and constraint changes are operators too}
  constraint/evolve/drop-old/readers-first: {title: A drop takes effect only once no registered reader relies on it, reason: consistent with readers-first}
  constraint/evolve/icmo: {title: Constraints added and dropped by their own typed operators, reason: operators carry constraints and constraint changes are operators too}
  constraint/evolve/policy/check: {title: Refuse the operator while any row violates the constraint, reason: a new constraint is validated against existing data}
  constraint/evolve/unmapped/refuse: {title: The operator is refused until the constraint is rewritten or dropped, reason: an operator that cannot carry a constraint is refused}
  constraint/exclusion: {title: No two rows of a scope stand in a given relation, reason: 'one general form, the assertion, with the other kinds as its common cases'}
  constraint/fd: {title: Functional dependencies not implied by a key, reason: 'one general form, the assertion, with the other kinds as its common cases'}
  constraint/form/empty: {title: A query that must return nothing, reason: 'a constraint is a query that must return nothing, in the query language, and its result is the violation report'}
  constraint/inclusion: {title: General inclusion dependencies between two queries, reason: 'one general form, the assertion, with the other kinds as its common cases'}
  constraint/overlap/equality-prefix: {title: 'Only equality fields plus one trailing range, probed per key', reason: exclusion over ranges needs a range index; an equality prefix partitions it}
  constraint/overlap/range-index: {title: An interval index over a constructor set for overlap probes, reason: exclusion over ranges needs a range index; an equality prefix partitions it}
  constraint/refine/write-only: {title: A check on write; reads see the base type, reason: refinement types would need an entailment procedure; the domain is checked on write}
  constraint/row-check: {title: Checks over one row's fields, reason: 'one general form, the assertion, with the other kinds as its common cases'}
  constraint/scope/ctor: {title: 'On one constructor, over its own fields', reason: 'scopes are constructor sets, as for indexes'}
  constraint/scope/ctor-set: {title: 'On a set of constructors, over fields they share', reason: 'scopes are constructor sets, as for indexes'}
  constraint/scope/match: {title: 'On the sum, as an exhaustive match giving one predicate per constructor', reason: 'scopes are constructor sets, as for indexes'}
  constraint/scope/new-ctor/arm-in-add: {title: The add operator supplies an arm for every match constraint, reason: 'adding a constructor must state its arm of every match-scoped constraint, as for queries'}
  constraint/timing/multi-assign: {title: 'At statement end, with multiple assignment for compound changes', reason: every statement boundary is a consistent state; multiple assignment removes the need to defer}
  derived/admit/bounded: {title: Refuse a stored query without an index-bounded incremental plan, reason: the same rule and compiler as constraints}
  derived/computed/expr-index: {title: 'An index over an expression, with no field', reason: virtual fields evaluated on read; expression indexes as derived B-trees}
  derived/computed/virtual: {title: 'A field defined by an expression over the row, evaluated on read', reason: virtual fields evaluated on read; expression indexes as derived B-trees}
  derived/defaults/backfill/separate: {title: The add operator states its own fill; the default governs later inserts only, reason: the add operator states its fill; the default governs later inserts}
  derived/defaults/on-relation: {title: 'Defaults on a relation variable, applied by inserts into it', reason: 'a default belongs to the insert into a relation variable, not to the value'}
  derived/event-sourcing/none: {title: No particular support, reason: an append-only relation of an event sum with stored views is already expressible; nothing is built in}
  derived/freshness/hotspot/blind-delta: {title: Maintenance writes are commutative increments that record no read, reason: 'maintenance increments commute and record no read, so writers to one group do not conflict'}
  derived/freshness/in-txn: {title: Maintained inside the writing transaction's change set, reason: serializable reads of a view beside its base need the view in the same change set}
  derived/ivm/delta-rules: {title: Compile per-relation delta queries over the change set and base indexes, reason: the constraint delta compiler already maintains views that must stay empty; stored views use it over base indexes}
  derived/ivm/state/base-indexes: {title: Delta queries probe base relations through their indexes; no private state, reason: 'delta queries probe base indexes, the same bound constraints are admitted under; no second state'}
  derived/materialize/storage-choice: {title: A derived relation is declared once and storing it is a physical choice, reason: a stored view denotes the same as its query; storing it is a physical choice maintained in the transaction}
  derived/mv-evolve/rebuild: {title: 'Rebuild the stored relation as a staged job, like an index build', reason: 'translate where the operator maps rows, rebuild as a staged job otherwise'}
  derived/mv-evolve/translate: {title: The operator maps the stored rows as it maps base rows, reason: 'translate where the operator maps rows, rebuild as a staged job otherwise'}
  derived/mv-integrity/constrained: {title: Constraints read the stored relation and its delta, reason: 'a stored view is a relation and carries what relations carry, except as a reference target'}
  derived/mv-integrity/indexed: {title: Indexes over constructor sets of the stored result, reason: 'a stored view is a relation and carries what relations carry, except as a reference target'}
  derived/mv-integrity/keyed: {title: 'A declared key on the stored result, checked as a constraint', reason: 'a stored view is a relation and carries what relations carry, except as a reference target'}
  derived/mv-storage/heaps: {title: 'Like a base relation, one heap per constructor with a union index', reason: a stored view is held like a base relation}
  derived/rules/none: {title: No triggers or rules; delete actions are the only engine-derived writes, reason: writes are change-set values; delete actions are the only engine-derived writes}
  derived/time-travel/basis/both: {title: 'Commit numbers, with a recorded map from wall time to commit number', reason: 'commit numbers, with a recorded map from wall time'}
  derived/time-travel/past-schema/as-written: {title: A past read is typed against the schema version current at that point, reason: 'a past read is typed against the schema version current then, as a bundle pinned to it'}
  derived/time-travel/retention/history-heap: {title: Vacuum moves retained dead versions to a history heap per constructor, reason: retained versions move to a history heap per constructor so current scans do not pay}
  derived/time-travel/window: {title: Any snapshot within a retention window, reason: 'versions already live in the heap; keeping them for a bounded window is a horizon setting, not a new store'}
  derived/transition/change-visible: {title: A constraint may read the inserted and deleted sets and the prior state, reason: constraints already run on the change set; letting them name it makes append-only and immutable checkable}
  derived/valid-time/pattern: {title: Valid time is a period field with an overlap exclusion constraint, reason: a period field with an overlap exclusion; no second notion of time in the model}
  derived/valid-time/temporal-fk/live: {title: Live with temporal references checked only by key, reason: 'valid time is a pattern; a covering reference is a constraint query the user writes, admitted when bounded'}
  encoding/native: {title: Sum is a first-class schema object enforced by the engine}
  encoding/native-cost/union-index: {title: Global index over every constructor's heap, reason: realizes ctor-set indexes over per-constructor heaps}
  engine/catalog-relations: {title: 'The catalog is sums in the engine, read and written by transactions', reason: 'the catalog is relations, queried with the same language'}
  engine/ddl-online-txn/staged-job: {title: 'The transaction commits the first state, a job runs the rest', reason: online schema states run as a staged job}
  engine/embedded-server: {title: Embeddable core with an optional server wrapping it, reason: 'an embedded core first, a server around it'}
  engine/guarantees-only: {title: 'Spec states guarantees over constructor relations, mechanism free', reason: the spec states guarantees over constructor relations; heaps and pages are implementation}
  engine/in-heap: {title: 'Every version a tuple in its constructor''s heap, vacuum removes dead ones', reason: dead versions in the heap are what batched per-sum vacuum collects}
  engine/online-states: {title: 'The operator passes through intermediate states, each compatible with the last', reason: 'staged schema states, the same discipline as readers-first'}
  engine/own-pager: {title: 'Own pager, heap files, B-trees and log', reason: 'the settled heap, TID and batched vacuum vocabulary is page-based'}
  engine/pinned-version: {title: 'Each transaction runs against the schema version of its snapshot, translated across versions', reason: staged states roll a change out; pinned versions keep serving queries checked against an older version}
  engine/redo-mvcc: {title: 'Redo-only WAL, old versions stay in heaps so no undo', reason: 'with in-heap versions, aborted work is invisible, so redo-only logging suffices'}
  engine/retag-chain/forward-pointer: {title: The old version points at the new one by constructor and TID, reason: an updater of the old version follows the chain into the new heap}
  engine/retag-fk/by-isolation: {title: Serializable read tracking catches the race, reason: serializability orders a retag against concurrent references}
  engine/retag-snapshot/two-entries: {title: 'Retag adds an entry for the new version, the old one stays until vacuum', reason: 'index entries are per version, so older snapshots still find the old one'}
  engine/retry/retry-block: {title: A transaction block is a retryable unit the client library reruns, reason: retrying is part of the client contract; read-only transactions can wait for a safe snapshot and never abort}
  engine/retry/safe-snapshot: {title: Declared read-only transactions take a safe snapshot and never abort, reason: retrying is part of the client contract; read-only transactions can wait for a safe snapshot and never abort}
  engine/serializable-only: {title: 'Every transaction is serializable, no weaker level', reason: 'one isolation level, serializable; constraints and retag reasoning assume it'}
  engine/ssi: {title: MVCC with serializable snapshot isolation, reason: serializable with readers that never block}
  engine/transactional-ddl: {title: Operators and data writes commit or roll back together, reason: a schema change is a write like any other}
  engine/wait-inserter: {title: 'Fetch the entry''s tuple through its constructor''s heap, wait on an in-progress writer', reason: 'the proven Postgres check, with each TID fetch routed by constructor id'}
  engine/write-skew/by-isolation: {title: Checks are reads of a serializable transaction, reason: serializability rules out write skew on at-least-one}
  evolution/add: {title: Add a constructor, reason: 'with sums closed, adding a constructor is the only way one grows'}
  evolution/breaks/invisible/registered: {title: 'Persisted, registered queries the database checks', reason: a query the database cannot see cannot be checked}
  evolution/breaks/recompile: {title: Recompile stored queries on schema change and report failures, reason: 'exhaustiveness is a type check, so a schema change re-checks dependents'}
  evolution/hoist/default: {title: Constructors lacking the field take a declared default, reason: the default is an explicit argument of the operator; dropping the hoisted field inverts it}
  evolution/in-log: {title: 'define, redefine and drop are operators in the log, rechecked like any change', reason: 'definitions change in the same log as data shape, so an add and the functions it breaks land together'}
  evolution/log-schema-only: {title: enable and deprecate are log operators; retire is a deployment command outside the log, reason: retirement is deployment state; an engine-written retire diverges the file's log from shipped migrations (spec/log-content)}
  evolution/merge: {title: Merge constructors into one, reason: 'the operator set is closed under inverses, so every change can be undone and versions translated'}
  evolution/merge-tag/discriminator: {title: Merged constructor keeps the old tag as a field, reason: 'keeping the tag makes merge the inverse of split, as inverse closure requires'}
  evolution/move-field: {title: Move a field between constructors or up to the sum, reason: 'the operator set is closed under inverses, so every change can be undone and versions translated'}
  evolution/nest: {title: 'Nest constructors under an intermediate sum, or un-nest', reason: 'the operator set is closed under inverses, so every change can be undone and versions translated'}
  evolution/old-readers/readers-first: {title: Require readers to upgrade before writers use the constructor, reason: an unknown constructor would bring back wildcards; writers never emit what readers cannot type}
  evolution/remove: {title: Remove a constructor, reason: 'the operator set is closed under inverses, so every change can be undone and versions translated'}
  evolution/remove-rows/deprecate: {title: 'Deprecate first, remove in a later version', reason: 'mirror of readers-first: writers stop before readers lose the constructor'}
  evolution/remove-rows/merge-first: {title: Require migrating its rows into another constructor first, reason: 'removing a populated constructor is merge then remove, each with its inverse'}
  evolution/remove-rows/restrict: {title: Refuse while rows or references exist, reason: remove applies only to an empty constructor; it never deletes data}
  evolution/rename: {title: Rename a constructor, reason: 'the operator set is closed under inverses, so every change can be undone and versions translated'}
  evolution/revert: {title: revert appends the inverse operators as a new migration, reason: undo appends inverses; the log only grows}
  evolution/scripted: {title: Hand-written migration scripts per change, reason: escape hatch for transforms no operator expresses}
  evolution/smo: {title: A closed language of schema modification operators, reason: typed operators with known inverses or a declared loss}
  evolution/split: {title: Split a constructor into several, reason: 'the operator set is closed under inverses, so every change can be undone and versions translated'}
  evolution/split-routing/match-route: {title: 'Route by a match on the old fields, checked exhaustive', reason: 'routing is a match, so the exhaustiveness check already proves it total'}
  evolution/types: {title: Operators act on a type and every relation variable of it changes, reason: a type is one definition; each relation variable states which of its constructor sets a new constructor joins}
  exec/arm-card/per-arm: {title: 'Each arm estimated on its own heap''s statistics, summed', reason: disjoint arms make a match cardinality the sum of its arms}
  exec/batch/arrow: {title: 'Arrow record batches, one schema per constructor', reason: a heap of one constructor yields batches of one schema with no validity bitmap}
  exec/flow/push: {title: 'Producers push into consumers, plans split into pipelines', reason: 'push-based pipelines, as compiled and morsel execution need'}
  exec/interleaved/sel-vectors: {title: Split each batch into one selection vector per tag, reason: selection vectors per constructor when the tag is known only per row}
  exec/join/cost: {title: The optimizer chooses per query between both, reason: distributing a join into arms or hoisting it is a cost decision}
  exec/join/rebuild/shared-build: {title: 'Build the other side''s hash table once, probe it from every arm', reason: one hash build shared by every arm}
  exec/model/adaptive: {title: Pipelines start in a bytecode interpreter and compile when it pays, reason: 'interpret first, compile hot registered queries'}
  exec/parallel/morsel: {title: A dispatcher hands morsels of each pipeline to worker threads, reason: morsel-driven parallelism}
  exec/parallel/small-heaps/pack: {title: Pack several small arm pipelines into one task, reason: small constructor heaps are packed into shared morsels}
  exec/plan-cache/at-register: {title: 'Planned at registration, plan stored in the catalog', reason: registered queries are planned and checked at registration}
  exec/probe/fuse: {title: 'One probe of the union index, then route each entry to its arm', reason: follows from query/native}
  exec/prune/compile: {title: At compile time from types and constant arms, reason: prune constructors from types at compile time and from parameters at start}
  exec/prune/init: {title: At executor start from parameter values, reason: prune constructors from types at compile time and from parameters at start}
  exec/replan/schema: {title: A change to any sum the plan reads, reason: replan on schema change and when statistics drift}
  exec/replan/stats: {title: Statistics drifting past a threshold, reason: replan on schema change and when statistics drift}
  exec/stats-form/sketch: {title: Online reservoir samples and mergeable distinct-count sketches, reason: 'mergeable sketches, since distinct counts across a sum do not add'}
  exec/stats/per-ctor: {title: Row count and per-field statistics for each constructor heap, reason: statistics per constructor heap}
  exec/value-sum/dense: {title: Tag vector plus offsets into compact per-constructor children, reason: value-level sums in vectors as Arrow dense unions}
  exec/value-sum/special/enum-tag: {title: A sum of nullary constructors as its tag vector alone, reason: 'Option as a validity bitmap, enums as bare tags, one-constructor sums as their payload'}
  exec/value-sum/special/one-ctor: {title: A one-constructor sum as its payload with no tag, reason: 'Option as a validity bitmap, enums as bare tags, one-constructor sums as their payload'}
  exec/value-sum/special/option-validity: {title: A two-constructor sum with one nullary constructor as a validity bitmap, reason: 'Option as a validity bitmap, enums as bare tags, one-constructor sums as their payload'}
  fk/retag/recheck: {title: Retag checks every to-ctor FK pointing at the row, reason: each FK sees the change in the relation it references}
  fk/subset/subset-index: {title: Union index over only the named constructors' heaps, reason: follows from ctor-set indexes}
  fk/to-ctor: {title: One specific constructor, reason: all are FKs to a constructor set}
  fk/to-subset: {title: A subset of constructors, reason: all are FKs to a constructor set}
  fk/to-sum: {title: The sum as a whole, reason: all are FKs to a constructor set}
  index/ctor-set: {title: Any set of constructors of one sum, reason: 'one mechanism: sum PK, constructor uniques and subset targets are all indexes over a constructor set'}
  interop/dump/data/arrow-ipc: {title: An Arrow IPC stream per constructor relation, reason: 'Arrow per constructor relation, text for review'}
  interop/dump/data/text-rows: {title: 'One row per line in the textual value syntax, per constructor relation', reason: 'Arrow per constructor relation, text for review'}
  interop/dump/logical: {title: A logical dump of the schema and the rows of every relation, reason: a physical copy is repl/backup; a logical dump crosses format versions}
  interop/dump/restore/offline-validate: {title: 'Rows are written unchecked into a database not yet open, then every constraint query runs once', reason: 'rows written into a closed database, then every constraint query runs once'}
  interop/dump/roundtrip/same-ids: {title: 'Equal relations, the same schema version, constructor ids and key values', reason: 'bundles pin versions and ids, so a restore must keep them'}
  interop/dump/schema/op-log: {title: 'The full operator log, replayed onto an empty database', reason: the schema is the fold of its log}
  interop/dump/scope/bundles: {title: Registered bundles and their version pins, reason: bundles pin the dump's versions; extensions by name and version}
  interop/dump/scope/extensions: {title: 'References to installed extensions by name and version, not their contents', reason: bundles pin the dump's versions; extensions by name and version}
  interop/ext-pkg/catalog-module: {title: 'An extension is a module whose definitions and Wasm code are catalog rows, installed by schema operators', reason: 'an extension is a module of definitions and Wasm code installed by schema operators, so it travels with the file and the log'}
  interop/foreign/file-scan: {title: 'Immutable files read as relations in ad hoc queries and loads, never stored as relation variables', reason: 'immutable files read in ad hoc queries and loads; never relation variables, so constraints and isolation keep their scope'}
  interop/foreign/integrity/no-constraints: {title: 'Constraints, keys and references may not mention an external relation', reason: 'constraints may not mention a file, and a transaction reads each file once at a recorded checksum'}
  interop/foreign/integrity/pinned-file: {title: A transaction reads each file once at a recorded version and checksum, reason: 'constraints may not mention a file, and a transaction reads each file once at a recorded checksum'}
  interop/foreign/typing/declared: {title: The user states a row type and each read checks the file against it, reason: 'queries are checked at registration, before files exist'}
  interop/formats/arrow-ipc: {title: Arrow IPC stream and file formats, reason: each is a reader for loads and a writer for export}
  interop/formats/csv: {title: CSV, reason: each is a reader for loads and a writer for export}
  interop/formats/flat-sums/tag-columns: {title: 'A tag column plus one column or struct per constructor, empty for the others', reason: tag plus per-constructor columns for tools; text cells for round trips}
  interop/formats/flat-sums/text-cell: {title: The field is one text cell in the textual value syntax, reason: tag plus per-constructor columns for tools; text cells for round trips}
  interop/formats/ndjson: {title: Newline-delimited JSON, reason: each is a reader for loads and a writer for export}
  interop/formats/parquet: {title: Parquet, reason: each is a reader for loads and a writer for export}
  interop/fts/derived: {title: An inverted index as a derived term relation with a B-tree; tokenizers are functions, reason: an inverted index is a derived term relation with a B-tree; tokenizers are functions}
  interop/index-am/derived: {title: A special index is a B-tree over a relation derived from the indexed one by a function, reason: 'special indexes are B-trees over relations derived by a function, so they inherit MVCC, WAL and the union index'}
  interop/index-am/fixed: {title: The engine ships a fixed set of index kinds, reason: 'special indexes are B-trees over relations derived by a function, so they inherit MVCC, WAL and the union index'}
  interop/json-map/external: {title: 'An object with one key, the constructor name, holding the payload', reason: 'one key naming the constructor, holding the payload'}
  interop/json/none: {title: No JSON type; JSON is only an import and export format, reason: sums replace schemaless payloads; JSON is only a format}
  interop/load/append-api: {title: The host program streams Arrow batches into a constructor relation, reason: 'a load is an insert from a query over the file, or a host stream of Arrow batches; both are change sets checked as one'}
  interop/load/check/build-time: {title: Uniqueness is checked while the index is built from sorted runs, reason: each delta query runs once as a join; uniqueness is checked while building from sorted runs}
  interop/load/check/set-at-a-time: {title: 'Each constraint''s delta query runs once over the whole delta, as a join', reason: each delta query runs once as a join; uniqueness is checked while building from sorted runs}
  interop/load/errors/reject-input: {title: Rows failing parse or routing go to a rejects relation; constraint violations abort, reason: parse and routing failures go to a rejects relation; constraint violations abort with the report}
  interop/load/index-path/bulk-build: {title: 'Into an empty constructor set, the index is built afterwards from sorted runs', reason: the planner picks per load}
  interop/load/index-path/retail: {title: Each row's entries are inserted as it is written, reason: the planner picks per load}
  interop/load/index-path/sorted-merge: {title: 'Into a populated index, the delta''s entries are sorted and inserted in key order', reason: the planner picks per load}
  interop/load/shape/per-ctor: {title: 'One file per constructor, each inserted into its constructor relation', reason: each external shape has a route onto constructors}
  interop/load/shape/tag-match: {title: A tag column routed to constructors by a match, reason: each external shape has a route onto constructors}
  interop/load/shape/tagged-text: {title: Each record carries its value in the textual value syntax, reason: each external shape has a route onto constructors}
  interop/load/shape/union-column: {title: An Arrow dense union column maps member by member onto constructors, reason: each external shape has a route onto constructors}
  interop/load/shape/unrouted/tag-parse: {title: 'The tag column is parsed into the sum''s tag type, then matched exhaustively', reason: the tag column parses into the sum's tag type; failures go to the rejects relation}
  interop/load/source: {title: An external file is a relation in a query; loading is an insert from it, reason: 'a load is an insert from a query over the file, or a host stream of Arrow batches; both are change sets checked as one'}
  interop/load/unit/staged: {title: 'Rows land in an unconstrained staging relation, then one insert moves them', reason: 'rows land unconstrained in staging, then one insert moves them as one change set checked set-at-a-time'}
  interop/load/wal/page-images: {title: Each newly filled heap and index page is logged once as a full image, reason: filled pages logged once as images; replicas get the rows}
  interop/migrate/dirty/fix-script: {title: 'The tool emits a repair query per violation class, run against the source first', reason: 'typed quarantine for repair, and repair queries per violation class'}
  interop/migrate/dirty/quarantine: {title: Violating source rows load into a typed quarantine relation for repair, reason: 'typed quarantine for repair, and repair queries per violation class'}
  interop/migrate/infer/declared: {title: The user names the tag column and the constructors' columns, reason: 'checks and profiles propose, the user declares'}
  interop/migrate/infer/from-checks: {title: CHECK constraints relating a tag column to nullness are read as the sum, reason: 'checks and profiles propose, the user declares'}
  interop/migrate/infer/profile: {title: Null co-occurrence in the data is mined for candidate constructors, reason: 'checks and profiles propose, the user declares'}
  interop/migrate/null/report: {title: The tool lists every constraint whose meaning changes under the mapping, reason: 'uniques over Some values, and a report of every constraint whose meaning changes'}
  interop/migrate/null/translate-unique: {title: A unique on a nullable column becomes a uniqueness over its Some values, reason: 'uniques over Some values, and a report of every constraint whose meaning changes'}
  interop/migrate/patterns/exclusive-arc: {title: Several nullable FKs with an exactly-one check become a sum, reason: each is a mapping rule the user confirms}
  interop/migrate/patterns/jsonb-tagged: {title: A jsonb column with a type field becomes a value-level sum, reason: each is a mapping rule the user confirms}
  interop/migrate/patterns/nullable-option: {title: A nullable column becomes an Option field, reason: each is a mapping rule the user confirms}
  interop/migrate/patterns/pg-inherit: {title: Postgres inheritance children become constructors of one sum, reason: each is a mapping rule the user confirms}
  interop/migrate/patterns/polymorphic: {title: A type plus id pair becomes a sum with one reference per constructor, reason: each is a mapping rule the user confirms}
  interop/migrate/patterns/sti: {title: A type column with per-type nullable columns becomes a relation-level sum, reason: each is a mapping rule the user confirms}
  interop/migrate/tool: {title: An offline tool reads a SQL catalog and data and proposes a schema and a load, reason: 'the SQL reader stays outside the engine, as there is no SQL surface'}
  interop/scalar/newtype-only: {title: 'Every type is a sum, record or newtype over the base types', reason: 'every type is a sum, record or newtype over base types, so key encoding and order are always derived'}
  interop/spatial/out: {title: Out of scope; geohash or S2 cells as B-tree keys only, reason: geohash or S2 cells as B-tree keys}
  interop/text/literal: {title: The query language's literal syntax is the text format, reason: 'one syntax for code, dumps, fixtures and errors'}
  interop/text/rename/versioned: {title: A text document states its schema version and the reader translates through the operator log, reason: text states its schema version and is translated through the operator log}
  interop/uda/fold-decl: {title: 'An aggregate declares a state, a step, a merge and a finish', reason: 'state, step, merge and finish run morsel-parallel'}
  interop/udf/abi/arrow: {title: A batch of Arrow arrays in linear memory per call, reason: a batch of Arrow arrays per call amortizes the boundary}
  interop/udf/host-fn: {title: Rust functions registered by the embedding program, reason: 'sandboxed modules in the catalog, and Rust functions from the embedding program'}
  interop/udf/identity/content-hash: {title: A function's identity includes a hash of its code; replacing it is a schema operator that rebuilds and rechecks dependents, reason: replacing code is a schema operator that rebuilds and rechecks dependents}
  interop/udf/purity/declared: {title: 'Signatures declare determinism; only deterministic functions may appear in keys, indexes and constraints', reason: 'Wasm gets no imports and canonical NaNs; host functions declare determinism and only deterministic ones reach keys, indexes and constraints'}
  interop/udf/purity/wasm-det: {title: 'WebAssembly modules get no imports and canonical NaNs, so they are deterministic by construction', reason: 'Wasm gets no imports and canonical NaNs; host functions declare determinism and only deterministic ones reach keys, indexes and constraints'}
  interop/udf/wasm: {title: Sandboxed WebAssembly modules stored in the catalog, reason: 'sandboxed modules in the catalog, and Rust functions from the embedding program'}
  interop/vector/dim-type/newtype-bytes: {title: A newtype over Bytes with a domain constraint on its length, reason: scalar types are newtypes over base types}
  interop/vector/scan-only: {title: A vector type and distance functions; exact search by scan only, reason: 'an approximate index has no predicate for serializable reads and index kinds are not pluggable; exact search by scan, approximate search outside fed by the change stream'}
  lang/abstraction/higher-order: {title: 'Functions over predicates and queries, normalised away before planning', reason: 'abstraction over predicates and queries, normalised away so the planner sees first-order queries'}
  lang/abstraction/ho-recursion/first-order-recursion: {title: Recursive definitions take only first-order arguments, reason: keeps normalisation total}
  lang/binder/ctor-row: {title: 'The constructor''s row type, a subtype of the sum', reason: 'the binder has the constructor row type, a subtype of the sum'}
  lang/collection/set-agg/over-binders: {title: 'Aggregates range over bindings, not projected values', reason: 'aggregates range over bindings, so equal projected values still count'}
  lang/collection/sets: {title: 'Sets, duplicates never exist', reason: 'results are relations, and relations are sets'}
  lang/defs-home/bundle: {title: Client bundles registered together with their queries, reason: schema functions are rechecked with the schema; client queries are registered in bundles}
  lang/defs-home/schema: {title: 'Schema objects, rechecked with the schema', reason: schema functions are rechecked with the schema; client queries are registered in bundles}
  lang/empty-agg/nonempty: {title: Groups are typed nonempty; Option only over arbitrary collections, reason: groups are nonempty by construction; Option only where emptiness is possible}
  lang/family/comprehension: {title: Comprehensions with clauses in binding order, reason: comprehensions over sets and nested results have the NRC and T-LINQ theory behind them}
  lang/group/ctor-key/tag-type: {title: Each sum has a derived tag type with one nullary constructor per constructor, reason: 'the tag of a sum is itself a nullary sum, usable as a key'}
  lang/group/into: {title: Group binds a key and the group's rows as a collection, reason: group binds a key and its rows as a nested collection}
  lang/inference/signatures: {title: Function and parameter signatures written; bodies inferred, reason: signatures are the interface; bodies are inferred}
  lang/keyset: {title: Pages continue after a key of the last row seen, reason: continue after a key}
  lang/nav/backlink-card/collection-typed: {title: A backlink has a collection type and is a type error as a scalar, reason: a backlink is a collection of referrers; nesting it is its meaning}
  lang/nav/backlinks: {title: Reverse navigation from a target to the rows referencing it, reason: paths and backlinks are sugar over generators on typed keys}
  lang/nav/joins: {title: Explicit generators joined by predicates, reason: paths and backlinks are sugar over generators on typed keys}
  lang/nav/path-option/option-typed: {title: The path yields an Option and must be matched, reason: an optional reference is an Option and is matched like any sum}
  lang/nav/paths: {title: Path navigation through foreign keys, reason: paths and backlinks are sugar over generators on typed keys}
  lang/nested-order/top-n: {title: 'A top n by key operator yielding a set, usable at any depth', reason: 'top n by a key is a deterministic set, so limits work at any depth without lists'}
  lang/nullary-fn: {title: A view is a parameterless schema function, reason: a view is a parameterless function; one concept}
  lang/option-params: {title: 'Option parameters whose None disables a filter, specialised at plan time', reason: optional filters are Option parameters; plans are specialised per present-parameter pattern}
  lang/order/result-only: {title: Order and limit only at the outermost result, reason: relations are sets; order is presentation of the final result}
  lang/outer/nested: {title: No outer join; the related rows are a nested collection, reason: no outer join; the missing side is an empty nested collection}
  lang/params/declared: {title: Parameters declared with types in the query signature, reason: parameters are part of a registered query signature}
  lang/patterns/failure-sharing/guard-vars/second-scope: {title: Check the guard under its own binder variables, reason: Keeps one binding site per variable as kernel.md section 2 states; no term copying.}
  lang/patterns/failure-sharing/retest: {title: 'Lower each arm to one condition and one body, retesting the pattern in the body', reason: 'Linear kernel terms for every pattern shape with no new kernel form and no change to any consumer: F appears once per arm, so the normaliser memo refusing draws and every tree walker stop mattering for matches. It is the backtracking-automaton trade (linear size, retested subterms), and the current lowering already retests the scrutinee per arm. memo cannot fix path-sensitive passes (delta guards, planner, code generator) without rebuilding join points inside each; join is the principled general form but costs every consumer plus disjunctive path conditions in the planner and incremental checks, for a problem the lowering can avoid creating.'}
  lang/patterns/failure-sharing/retest-cost/live: {title: Live with one repeated total test per nested constructor, reason: One tag read per nested constructor in the arm that matches; failing arms cost what they cost today.}
  lang/patterns/failure-sharing/unreachable-meaning/live: {title: Restate Unreachable as a position the checker proves unreached, reason: One sentence in the design file; reaching it stays a broken invariant.}
  lang/patterns/guard-cover/ignore-guarded: {title: Guarded arms count for nothing toward coverage, reason: coverage is decided without evaluating guards}
  lang/patterns/guards: {title: Guards on arms, reason: the standard pattern language}
  lang/patterns/literals: {title: Literal patterns on scalars, reason: the standard pattern language}
  lang/patterns/nested: {title: Nested constructor patterns on value-level sum fields, reason: the standard pattern language}
  lang/patterns/or-binder/ctor-set: {title: The binder has the constructor-set type of the alternatives, reason: 'an or-pattern binder has the constructor-set type, the same sets indexes and FKs use'}
  lang/patterns/or-patterns: {title: Or-patterns over several constructors, reason: the standard pattern language}
  lang/patterns/record: {title: Record patterns binding fields by name, reason: the standard pattern language}
  lang/record-poly/nominal: {title: Functions take named relations and constructors only, reason: consistent with nominal types}
  lang/recursion/fix: {title: A fixpoint binder inside the comprehension language, reason: recursion stays inside the one language as a fixpoint binder}
  lang/recursion/negation/stratified: {title: Negation and aggregation only of lower strata, reason: negation and aggregation of lower strata keep fixpoints well defined}
  lang/registered/named: {title: 'Named queries with signatures in a catalog, called by name', reason: 'a registered query is a named, typed catalog object'}
  lang/results/nested: {title: 'Records may hold collections, nested to any depth', reason: 'NRC results: related rows nest instead of padding'}
  lang/sum-order/declared: {title: Each sum declares whether and how it is ordered, reason: order is not intrinsic to a sum; a sum states it}
  lang/wildcard/scalars-only: {title: Wildcards only on scalars; every constructor of a sum named, reason: 'exhaustive means every constructor named, so adding one breaks every match and recompile reports it'}
  lang/wildcard/verbosity/adhoc-wildcard: {title: Wildcards over sums are allowed in the ad hoc role only, reason: ad hoc queries carry no guarantee and are never rechecked}
  lang/wildcard/verbosity/set-patterns: {title: A pattern may name a named constructor set, reason: 'the decision a wildcard would make silently is made once, where the constructor is added, by joining a named set'}
  layout/alloc/bitmap: {title: One bit per page or block in bitmap pages, reason: a page allocation bitmap in the single file}
  layout/checksum/in-page-32: {title: A 32-bit CRC-32C in the page header, reason: a 32-bit checksum in every page}
  layout/endian/little: {title: Little-endian everywhere in pages and rows, reason: rows little-endian; index keys are order-preserving regardless}
  layout/field-order/packed: {title: Ordered by alignment to remove padding with a field-id to offset map in the catalog, reason: 'one schema per heap, so fields can be reordered for alignment'}
  layout/files/shrink/reuse-only: {title: Freed pages go to the free list and the file never shrinks, reason: freed pages are reused; the file does not shrink without an explicit compaction}
  layout/files/single-file: {title: One database file holding every heap and index as page sets, reason: an embedded database is one file whatever its schema}
  layout/fill/fsm: {title: A free space map with a coarse free-space value per page, reason: a free space map per heap}
  layout/format-version/read-old: {title: Every release reads every earlier format and writes the newest, reason: new binaries read old formats}
  layout/heap-format/page-version: {title: Each page has one format version and pages are converted on write, reason: 'each page states the row format it holds, so evolution rewrites lazily'}
  layout/large/retag/per-sum: {title: 'One out-of-line store per sum, so pointers survive a move', reason: 'the large-value store belongs to the sum, so a retag moves no out-of-line value'}
  layout/large/toast: {title: Chunks in a side relation behind an inline pointer, reason: large values out of line in a per-heap side store}
  layout/slots/by-schema: {title: Fixed slots for constructors with only fixed-width fields and slotted pages otherwise, reason: 'fixed slots for heaps whose constructor has only fixed-width fields, slotted pages otherwise'}
  layout/succ/global-tid: {title: The successor link holds a file-global TID; a moved flag marks a cross-heap successor, reason: 'page ids are file-global, so next names a cross-heap successor with no probe; key relookup is ambiguous after an A to B to A retag in one transaction'}
  layout/tag-value/catalog-id: {title: The constructor's global catalog id, reason: 'constructor ids are deterministic in log order and shared by instantiations, so one id space serves wire, heaps and stored value tags (spec/ctor-ids)'}
  layout/torn/fpw: {title: Log a full page image on the first change after each checkpoint, reason: 'full-page images in the redo log, as Postgres'}
  layout/tuple-header/lean: {title: Fixed 24-byte header with 64-bit xids and no attribute count or header length, reason: no null bitmap or attribute count with one schema per heap and no NULL}
  layout/var-fields/offset-table: {title: 'An array of end offsets after the fixed part, one per variable field', reason: variable-width fields located through an offset table}
  layout/vsum-tag/varint: {title: A variable-length integer, reason: never-reused tags outgrow a fixed width}
  layout/vsum/by-shape: {title: 'Max-payload when every constructor is fixed width and small, tag-var otherwise', reason: a value-level sum is stored in the layout its shape allows}
  layout/vsum/widen/rewrite: {title: 'The change rewrites every heap holding the sum, through the heap format change', reason: widening a value-level sum rewrites lazily through page versions}
  model/anon-results/anon-records: {title: Results get anonymous structural record types, reason: 'query results have no declared name, so their record types are structural'}
  model/bytes: {title: Byte strings, reason: the minimum base set; Bool is a prelude sum}
  model/composite: {title: A key may span several fields, reason: 'any value type may form a key; facets need (owner, tag) keys'}
  model/decimal: {title: Exact decimal with declared scale, reason: the minimum base set; Bool is a prelude sum}
  model/declared-key: {title: Identity is a declared key over fields and a key field may be engine-generated, reason: identity is a key value; an engine-generated key field is still a field}
  model/deferred-check: {title: Any reference graph and references are checked at the end of a unit holding the whole cycle, reason: cycles are created in one multiple assignment and checked at its end}
  model/dup-arms/dedupe: {title: Results are deduplicated after the union, reason: 'a union of sets is a set; deduplication is the semantics, not a fix'}
  model/enum-order/derived-order: {title: A sum is ordered only when it declares an order, reason: follows from declared ordering}
  model/float: {title: IEEE 754 binary floats, reason: the minimum base set; Bool is a prelude sum}
  model/inline-records: {title: A field's type may be any record type and is stored inline, reason: 'a record is a one-constructor sum, and value-level sums may be fields'}
  model/int: {title: Fixed-width signed integers, reason: the minimum base set; Bool is a prelude sum}
  model/newtype: {title: A distinct nominal type over a base type with explicit conversion, reason: a domain is a distinct nominal type}
  model/no-collections: {title: No collection-valued fields and many-valued facts are child relations, reason: 'many-valued facts are relations, where keys and FKs apply'}
  model/nominal: {title: Every record and sum type is equal only to itself by declared name, reason: schema operators act on declared names; types are their names}
  model/nullary-sum: {title: An enum is a sum whose constructors have no fields, reason: an enum is a sum; no second kind}
  model/one-ctor-sum: {title: Every relation is a sum and a plain relation has one constructor, reason: 'one concept: a plain relation is a sum with one constructor'}
  model/one-key: {title: The sum declares a key that every constructor carries, reason: one key across constructors keeps identity through retag and gives FKs to the sum a target}
  model/param-evolution/live: {title: Live with one operator reaching every instantiation, reason: an operator on a parametric sum reaching every instantiation is parametricity}
  model/prelude-bool: {title: Bool is a prelude sum False or True, reason: the minimum base set; Bool is a prelude sum}
  model/record-is-ctor: {title: A record type is a sum with one constructor, reason: a record is a one-constructor sum; no second type former}
  model/record-key: {title: A key field may have a record type, reason: 'any value type may form a key; facets need (owner, tag) keys'}
  model/ref-in-key: {title: A key may include a reference to another relation, reason: 'any value type may form a key; facets need (owner, tag) keys'}
  model/ref-subtyping/ctor-set-subsumption: {title: ref S is a subtype of ref T whenever S is a subset of T, reason: 'Key S widens to Key T when S is a subset of T, as constructor sets do'}
  model/refs-top-level: {title: References are only direct fields of a relation's constructors, reason: integrity lives on relations; values carry no references}
  model/relation-recursion-only: {title: Value types are non-recursive and recursion goes through references between rows, reason: 'recursive data is rows referencing rows, under integrity'}
  model/set-of-type: {title: A relation is a variable holding a set of values of a declared type, reason: a type and a variable holding a set of its values are distinct concepts}
  model/sets: {title: Every relation is a set and every query result is a set, reason: relations are sets; duplicates have no meaning}
  model/sum-header: {title: Common fields are declared once on the sum, reason: 'common fields are a fact about the sum, stated once'}
  model/sum-results: {title: A query result may be a sum whose constructors are its arms, reason: 'closure: a query result is of the same kind as a stored relation'}
  model/sum-valued-key: {title: A key field may have a value-level sum type, reason: 'any value type may form a key; facets need (owner, tag) keys'}
  model/text: {title: UTF-8 text, reason: the minimum base set; Bool is a prelude sum}
  model/time: {title: Instants and civil dates and times as distinct types, reason: the minimum base set; Bool is a prelude sum}
  model/typed-key: {title: A field holds the target key as a visible value whose type names the target constructor set, reason: 'with declared keys a reference is the target key, typed by its constructor set'}
  model/user-params: {title: Users declare parametric sums and fields instantiate them, reason: 'Option and Result are library sums, not engine built-ins'}
  mvcc/background: {title: Periodic background passes per sum, reason: 'per-sum background vacuum, plus opportunistic page pruning on access'}
  mvcc/ctor-tagged: {title: 'The lock carries the scanned constructor set, an insert conflicts only if its constructor is in it', reason: constructor membership is one more predicate conjunct of a range lock}
  mvcc/hot/per-heap: {title: An update is heap-only when no index whose constructor set contains this heap keys a changed field, reason: whether an update is heap-only is decided per heap from the indexes covering it}
  mvcc/link/global-tid: {title: A retag stores the successor TID in next; page ids are file-global, reason: 'page ids are file-global, so next names a cross-heap successor with no probe; key relookup is ambiguous after an A to B to A retag in one transaction'}
  mvcc/long-reader/ctor-scoped: {title: A transaction whose constructor relations are known holds back only their heaps, reason: 'registered queries name the constructor relations they read, so a long reader pins only those heaps'}
  mvcc/next-key: {title: Lock the key ranges between the entries the scan saw, reason: next-key locking is precise for ranges on the union index}
  mvcc/o2n: {title: 'Oldest to newest, each version links forward to its successor', reason: as Postgres; a new version is appended and found by key}
  mvcc/on-access: {title: Any reader or writer holding a page or chain prunes what it can there, reason: 'per-sum background vacuum, plus opportunistic page pruning on access'}
  mvcc/retag-phantom/as-insert: {title: The retag's index writes are checked against range locks as inserts of the new constructor, reason: the new entry of a retag is checked against range locks like an insert}
  mvcc/split-locks/promote-heaps: {title: Tuple and page locks on the rewritten heaps become relation locks on every heap the operator writes, reason: a split or merge carries SIREAD locks to the new heaps}
  mvcc/status/clog-lookup: {title: 'Every visibility check consults the commit log, versions are never touched by readers', reason: 'no hint bits, so reads never write pages and checksums stay simple'}
  mvcc/version-tid: {title: 'The physical slot of a version, new entries when a version moves page', reason: 'entries point at versions, consistent with two entries per retag'}
  mvcc/vm-per-heap: {title: 'Each constructor heap keeps a visibility map, consulted at the entry''s constructor and block', reason: 'the entry names the constructor, which selects the heap visibility map'}
  mvcc/watermark: {title: Older than the oldest snapshot still running, reason: one oldest-snapshot watermark}
  mvcc/xid-clog: {title: 'Writer transaction ids in the version, commit status in a commit log, snapshot as xmin, xmax and in-progress list', reason: 'transaction ids with a commit log, as Postgres'}
  mvcc/xid-width/xid64: {title: '64-bit ids in every version, never wrapping', reason: 'no wraparound, so no freezing'}
  net/adhoc/principal: {title: Ad hoc text is accepted only from principals holding an ad hoc grant, reason: ad hoc text is accepted only from principals holding the ad hoc grant}
  net/admission/classes: {title: 'Priority classes, ad hoc below registered', reason: 'bounded connections, a bounded queue answered with Overloaded, ad hoc below registered'}
  net/admission/connections: {title: A cap on connections and streams, reason: 'bounded connections, a bounded queue answered with Overloaded, ad hoc below registered'}
  net/admission/queue: {title: 'Calls wait in a bounded queue; beyond it, Overloaded with a retry hint', reason: 'bounded connections, a bounded queue answered with Overloaded, ad hoc below registered'}
  net/authn/mtls: {title: Client certificates mapped to principals, reason: 'passwords, certificates and Unix peer credentials per listener'}
  net/authn/peer: {title: OS peer credentials on Unix sockets, reason: 'passwords, certificates and Unix peer credentials per listener'}
  net/authn/scram: {title: SCRAM-SHA-256 with channel binding, reason: 'passwords, certificates and Unix peer credentials per listener'}
  net/base/own: {title: An own framed protocol over a byte stream, reason: 'request-id framed protocol; nothing reused fits stable constructor tags, credit flow and in-band cancel together'}
  net/bind/connection: {title: The handshake binds the connection to one bundle at its version, reason: 'the handshake binds one bundle at its version, which renews its lease'}
  net/call/ordinal: {title: 'A small id the engine assigned at registration, returned to codegen', reason: 'a small id assigned at registration, returned to codegen'}
  net/cancel/in-band: {title: A Cancel frame on the same connection names the stream, reason: 'streams are multiplexed, so a Cancel frame names one'}
  net/compression/frame: {title: A frame flag and a codec negotiated at Hello, reason: a frame flag and codec negotiated at Hello covers rows and Arrow alike}
  net/concurrency/hol/chunked: {title: Bound frame size and interleave chunks fairly across streams, reason: bounded frames interleaved fairly across streams}
  net/concurrency/multiplexed: {title: 'Independent concurrent calls and transactions, answered as each finishes', reason: independent calls and transactions per connection; no pooler needed}
  net/errors/sum: {title: 'Errors are values of a catalog sum, encoded with stable tags like any result', reason: errors are values of a catalog sum with stable tags}
  net/feed/changes: {title: 'A feed of committed change sets per constructor, resumable from a log position', reason: 'the change set is already the unit; feeds are owned by bundles, so readers-first covers them'}
  net/feed/retain/checkpoint: {title: 'The server keeps each named consumer''s position, as a replication slot', reason: 'named positions bounded by a window, then resnapshot'}
  net/feed/retain/window: {title: Positions older than a retention window are refused; the consumer resnapshots, reason: 'named positions bounded by a window, then resnapshot'}
  net/feed/version/bundle: {title: A subscription belongs to a bundle and is encoded at its pinned version, reason: 'a subscription is encoded at its bundle''s pinned version, so readers-first covers it'}
  net/flow/credit: {title: The client grants credit in bytes or batches per stream; the server sends up to it, reason: credit per stream bounds server buffering under multiplexing}
  net/flow/held/idle-limit: {title: A stream idle past a limit is cancelled with a typed error, reason: a stream idle past a limit is cancelled with a typed error}
  net/interactive/time-limit: {title: A transaction open past a limit is aborted, reason: registered transactions are the network path; interactive ones are bounded}
  net/nested/inline: {title: Each row carries its nested collections inline, reason: rows nest inline; in Arrow results that is List of Struct}
  net/parity/mirror: {title: Each protocol message is an in-process API call serialized; the server is a client of the library, reason: each message is an in-process call serialized; the server is a library client}
  net/pool/none-needed: {title: Connections are cheap; no pooler between client and server, reason: stateless async sessions are cheap}
  net/push/frames: {title: 'Typed push frames on a reserved stream, at any time', reason: typed pushes on a reserved stream}
  net/results/arrow: {title: 'Arrow IPC record batches, one schema per result', reason: 'rows for small results and simple drivers, Arrow for analytics; the client picks per call'}
  net/results/rows: {title: Row-wise binary records in the signature's field order, reason: 'rows for small results and simple drivers, Arrow for analytics; the client picks per call'}
  net/results/tag-width/type-map: {title: The schema maps each stable id to a type code via the union's typeIds, reason: the stream schema maps each stable id to an Arrow type code}
  net/retry/server: {title: 'The server reruns it before replying, up to a limit', reason: 'a registered transaction has no outside effects, so the server reruns it before replying'}
  net/server/async: {title: An async I/O front end; calls become tasks on the morsel scheduler, reason: calls become tasks on the morsel scheduler}
  net/session/stateless: {title: 'None; every call is self-contained, a transaction is named by an id', reason: every call is self-contained; an interactive transaction is named by id}
  net/task-boundary/owned: {title: An engine-owned resumable task and result handle independent of the socket runtime, reason: 'Engine ownership keeps authority, snapshot lifetime, cancellation and retries under one runtime-independent contract. Resumable waits and bounded production are required to preserve shared-worker liveness and actual credit suspension; a blocking wrapper alone does not provide those guarantees.'}
  net/task-resumption-cost/implement: {title: Add resumable engine waits and bounded result production before claiming server credit support, reason: 'Engine ownership keeps authority, snapshot lifetime, cancellation and retries under one runtime-independent contract. Resumable waits and bounded production are required to preserve shared-worker liveness and actual credit suspension; a blocking wrapper alone does not provide those guarantees.'}
  net/tls/direct: {title: TLS from the first byte on network transports with ALPN; plaintext only on Unix sockets, reason: TLS from the first byte with ALPN; no upgrade path to stuff}
  net/transport/tcp: {title: TCP, reason: one framed protocol over each byte stream}
  net/transport/unix: {title: Unix domain sockets for same-host clients, reason: one framed protocol over each byte stream}
  net/types/signature: {title: Fixed by the registered signature at the bound version; values carry no type information, reason: types come from the registered signature at the bound version}
  net/unknown/idempotency: {title: Each write call carries a client request id the engine deduplicates, reason: 'write calls carry a request id the engine deduplicates, so a retry after a lost acknowledgement applies once'}
  net/version/range: {title: The client offers a min and max version; the server picks one or refuses, reason: the client offers min and max; the server picks or refuses}
  ops/admin/actions/commands: {title: Actions are a small set of statements beside queries and writes, reason: 'a few statements beside queries and writes; they are effects, not data'}
  ops/admin/cli: {title: One command-line tool that works on a closed file and on a running server, reason: one CLI over a closed file or a server; operations are also statements for a privileged role}
  ops/admin/in-language: {title: 'Operations are expressible in the query language, by a privileged role', reason: one CLI over a closed file or a server; operations are also statements for a privileged role}
  ops/admin/role/capabilities: {title: 'Separate privileges to monitor, cancel, maintain and configure', reason: 'monitor, cancel, maintain and configure are separate privileges'}
  ops/admission/grant: {title: A call is admitted when its plan's memory grant fits the query budget, reason: 'a call runs when its plan''s memory grant fits the budget, which bounds the sum over concurrent queries'}
  ops/admission/queue: {title: 'Work queues per resource ordered by priority, then transaction start', reason: waiting calls queue by priority class then transaction start; the grant decides when the head runs}
  ops/cancel/cancel: {title: Cancel the running call; the transaction stays open and may roll back, reason: 'cancel a call, end a session, or cancel by token in process'}
  ops/cancel/terminate: {title: End the session and abort its transaction, reason: 'cancel a call, end a session, or cancel by token in process'}
  ops/cancel/token: {title: The embedded API takes a cancellation token per call, reason: 'cancel a call, end a session, or cancel by token in process'}
  ops/catalog-release/catalog-smo: {title: 'Catalog changes in a release are schema operators, applied readers-first to bundles that read the catalog', reason: the engine is a writer of catalog sums; releases ship catalog operators applied readers-first like any schema change}
  ops/check/cross: {title: Every live version has its index entries and every entry routes to an existing version of the constructor it names, reason: 'pages, index-to-heap routing, every constraint as a denial query, and a background scrub'}
  ops/check/logical: {title: Every admitted constraint is re-run as its denial query over the whole database, reason: 'pages, index-to-heap routing, every constraint as a denial query, and a background scrub'}
  ops/check/scrub: {title: A background pass reads every page on a schedule so latent errors surface before they are needed, reason: 'pages, index-to-heap routing, every constraint as a denial query, and a background scrub'}
  ops/check/structural: {title: 'Every page checksum and B-link invariant, as amcheck', reason: 'pages, index-to-heap routing, every constraint as a denial query, and a background scrub'}
  ops/compact/offline-copy: {title: 'Write every live version into a new file and swap, with the database closed', reason: copy into a new file closed or while serving}
  ops/compact/online-copy: {title: 'Copy into a new file while serving, replay the log written meanwhile, then switch', reason: copy into a new file closed or while serving}
  ops/config/derived: {title: 'Resource sizes derived from the host by default, overridable; no tuning knobs', reason: behaviour is fixed; resource sizes are derived from the host and overridable}
  ops/config/home/process/split: {title: Only file settings are writable relations; open-time settings are shown read-only, reason: only file settings are writable relations; open-time settings are read-only rows}
  ops/config/home/relation: {title: 'Settings are a system relation, read by queries and changed by writes', reason: persisted settings are one more catalog relation}
  ops/config/scope/file: {title: Persisted in the file and changed by a transaction, reason: a setting lives with what it must agree with}
  ops/config/scope/format: {title: Fixed when the file is created and recorded in its header, reason: a setting lives with what it must agree with}
  ops/config/scope/open: {title: Given when the file is opened; fixed for the process, reason: a setting lives with what it must agree with}
  ops/config/scope/session: {title: 'Set per session, transaction or call', reason: a setting lives with what it must agree with}
  ops/conflicts/counters: {title: Serialization failures and retries counted per registered query and constructor heap, reason: conflicts are logged as a system relation and counted per query and heap; the abort itself names only the caller's side}
  ops/conflicts/leak/admin-only: {title: 'Full detail only in the system relation, readable by a monitoring role', reason: 'full detail only in the system relation, for a monitoring capability'}
  ops/conflicts/log: {title: 'A bounded log of conflicts, queryable as a system relation', reason: conflicts are logged as a system relation and counted per query and heap; the abort itself names only the caller's side}
  ops/downgrade/finalize: {title: A new release writes only the old format until an operator finalizes the upgrade, reason: 'a new release writes the old format until finalized, so trying a release is reversible'}
  ops/explain/analyze: {title: explain analyze runs the query and reports actual rows and time per node, reason: a plan is a catalog value shown with its arms and pruned constructors}
  ops/explain/form/document: {title: Text for people and a versioned self-describing document for tools, reason: text for people and a versioned document for tools; plan operator kinds change every release}
  ops/explain/plan: {title: explain returns the chosen plan without running the query, reason: a plan is a catalog value shown with its arms and pruned constructors}
  ops/explain/stored: {title: The stored plan of a registered query is read from the catalog, reason: a plan is a catalog value shown with its arms and pruned constructors}
  ops/expose/callback: {title: The embedded library hands counters to the host through an API, reason: statistics are relations in the language; OTel for servers; a callback for embedders}
  ops/expose/otel: {title: The server exports OpenTelemetry metrics and traces, reason: statistics are relations in the language; OTel for servers; a callback for embedders}
  ops/expose/relations: {title: Statistics are system relations queried with the language, reason: statistics are relations in the language; OTel for servers; a callback for embedders}
  ops/expose/volatile/volatile-kind: {title: 'A distinct relation kind outside serializable history, unusable in constraints and writes', reason: 'statistics are a relation kind outside serializable history, unusable in constraints and writes'}
  ops/halt-scope/heap: {title: The constructor heap or index holding the page is quarantined; queries not reading it continue, reason: a constructor heap is the blast radius; queries not reading it continue}
  ops/limits/deadline: {title: The caller sends a deadline with each call and the engine stops at it, reason: 'limits per statement, transaction and idle time; deadlines per call; registered queries may declare their own'}
  ops/limits/declared: {title: A registered query declares its own limit in its bundle, reason: 'limits per statement, transaction and idle time; deadlines per call; registered queries may declare their own'}
  ops/limits/idle: {title: A limit on a transaction left open with nothing running, reason: 'limits per statement, transaction and idle time; deadlines per call; registered queries may declare their own'}
  ops/limits/statement: {title: A limit on each query or write execution, reason: 'limits per statement, transaction and idle time; deadlines per call; registered queries may declare their own'}
  ops/limits/transaction: {title: A limit on a transaction from begin to commit, reason: 'limits per statement, transaction and idle time; deadlines per call; registered queries may declare their own'}
  ops/limits/watermark/report: {title: 'The engine names the transaction or bundle holding the oldest snapshot, per heap', reason: name the holder per heap; killing it is an operator decision}
  ops/plan-release/stamp-replan: {title: Each stored plan carries the engine release; a new binary replans on open, reason: plans carry the engine release; a new binary replans on open}
  ops/plan-stability/last-good: {title: The engine reverts to the previous plan when a replan regresses measured cost, reason: operators pin plans; the engine reverts a replan that regresses measured cost}
  ops/plan-stability/pin: {title: An operator pins a plan; statistics drift no longer replans it, reason: operators pin plans; the engine reverts a replan that regresses measured cost}
  ops/promise/catalog: {title: System relations change only through catalog operators bundles can pin, reason: the four things a client or file depends on each carry a stated promise}
  ops/promise/editions: {title: A bundle states the language edition it is written in; every release compiles every edition, reason: the four things a client or file depends on each carry a stated promise}
  ops/promise/file: {title: Every release reads every file any earlier release wrote, reason: the four things a client or file depends on each carry a stated promise}
  ops/promise/protocol: {title: A stated window of client releases keeps working against a new server, reason: the four things a client or file depends on each carry a stated promise}
  ops/protocol/negotiate: {title: The client states a protocol version; the server answers with the highest it supports, reason: same as net/version/range}
  ops/query-stats/by-name: {title: Registered queries by bundle and name; ad hoc queries by a fingerprint of their checked form, reason: registered queries already have identity; ad hoc ones are fingerprinted by checked form}
  ops/quota/budget: {title: A bundle has a share of query memory and CPU time, reason: bundles declare a priority class and a share of memory and CPU}
  ops/quota/priority: {title: A bundle declares its priority class for admission, reason: bundles declare a priority class and a share of memory and CPU}
  ops/reindex/offline: {title: Rebuild with writes to the sum blocked, reason: online from sorted runs per heap; offline when writes may stop}
  ops/reindex/online: {title: 'Build a new index from sorted runs of each heap while writes continue, then swap', reason: online from sorted runs per heap; offline when writes may stop}
  ops/stats-history/windowed: {title: Counters aggregated in time windows and kept in the file for a retention period, reason: windows kept in the file for a retention period}
  ops/tracing/operator-spans: {title: 'Child spans for planning, each pipeline, commit and each wait', reason: 'spans named by registered query, children per stage, caller context carried'}
  ops/tracing/propagate: {title: The protocol carries the caller's trace context to the engine, reason: 'spans named by registered query, children per stage, caller context carried'}
  ops/tracing/query-span: {title: 'One span per execution, named by the registered query', reason: 'spans named by registered query, children per stage, caller context carried'}
  ops/vacuum/progress: {title: 'A progress relation per running vacuum, by sum, heap and phase', reason: 'progress per sum, heap and phase; start now; a budget'}
  ops/vacuum/throttle: {title: Vacuum runs under an I/O and CPU budget that operators set, reason: 'progress per sum, heap and phase; start now; a budget'}
  ops/vacuum/trigger: {title: An operator can start vacuum of one sum now, reason: 'progress per sum, heap and phase; start now; a budget'}
  ops/waits/activity: {title: A live sessions relation whose state is a closed sum of wait reasons, reason: 'wait reasons are a closed sum, live, sampled and cumulative'}
  ops/waits/cumulative: {title: 'Time and count per wait reason, cumulative and per registered query', reason: 'wait reasons are a closed sum, live, sampled and cumulative'}
  ops/waits/sampled: {title: Session states sampled at an interval into a bounded history, reason: 'wait reasons are a closed sum, live, sampled and cumulative'}
  project/arch/alloc/budgeted: {title: Fixed page pool; per-query arenas allocated on demand within a budget, reason: fixed page pool; per-query arenas on demand within a budget}
  project/arch/assertions/tiered: {title: Cheap assertions always; expensive checks in debug and simulation builds, reason: cheap assertions always; expensive checks in debug and simulation}
  project/arch/crates/by-boundary: {title: Crates only at stability or dependency boundaries, reason: crates only where stability or dependencies differ}
  project/arch/deps/allowlist: {title: A short audited allowlist; each dependency justified, reason: 'short audited allowlist; nothing that brings its own threads, clock or randomness into the simulator'}
  project/arch/deps/escape/no-ambient: {title: 'Admit only dependencies that take no ambient time, threads or randomness', reason: 'admit only dependencies with no ambient time, threads or randomness'}
  project/arch/errors/checker-guarantee/fail-statement: {title: Fail the statement with an internal error; the process and the transaction stay up, reason: 'an invariant the checker guarantees about a submitted program fails the statement with an internal error when broken, and the process keeps running; the transaction stays open. Invariants over engine state still crash and recover from the log. A client receives the failure as an Internal { what: Text } constructor of the statement error sum'}
  project/arch/errors/fatal-split: {title: Operating errors as Result; invariant violations crash and recover from the log, reason: operating errors are Result; invariant violations crash and recover from the log}
  project/arch/msrv/pinned: {title: 'A pinned toolchain, raised deliberately; the MSRV is that version', reason: a pinned toolchain raised deliberately}
  project/arch/runtime/own-loop: {title: 'The engine''s own event loop per thread, swapped for the simulator in tests', reason: the simulator must control every thread and completion}
  project/arch/unsafe/confined: {title: Unsafe only in one small module or crate with a stated contract; forbidden elsewhere, reason: 'unsafe lives in one crate with a stated contract, tested under Miri where Miri runs'}
  project/arch/unsafe/miri-io/sim-storage: {title: Run the engine under Miri over the simulated storage, reason: Miri over simulated storage; the system module natively with sanitizers}
  project/arch/unsafe/miri-io/sys-tests: {title: Test the unsafe system module natively with sanitizers, reason: Miri over simulated storage; the system module natively with sanitizers}
  project/concurrency/app-threads: {title: Tens of threads of one application, reason: 'embedded first: one process owns the file; other processes use the server'}
  project/docs-site/mdbook: {title: mdBook with generated navigation and an adt Highlight.js grammar, reason: 'The real-file prototype preserves links and anchors, supplies search and adt highlighting, and fits the Rust/Nix environment with less custom integration and dependency maintenance than Starlight or Pandoc. Hosting remains separate.'}
  project/nongoals/columnar-store: {title: No columnar storage format, reason: non-goals for an embedded engine competing with SQLite}
  project/nongoals/distributed: {title: No sharding or distributed transactions, reason: non-goals for an embedded engine competing with SQLite}
  project/nongoals/multi-process: {title: No several OS processes writing one database file, reason: non-goals for an embedded engine competing with SQLite}
  project/nongoals/sql-compat: {title: 'No SQL dialect, no Postgres or SQLite wire compatibility', reason: non-goals for an embedded engine competing with SQLite}
  project/oracle/float/tolerance: {title: Compare float results within a bound derived from the input size, reason: compare within a bound from input size}
  project/oracle/rust-reference: {title: 'A naive interpreter in Rust over in-memory sets, in the repository', reason: a naive interpreter over in-memory sets answers what any generated query should return}
  project/oracle/shared-front/typed-input: {title: Both consume generated typed kernel terms; the front end is tested separately, reason: both consume generated typed kernel terms; the front end is fuzzed separately}
  project/scale/out-of-core: {title: Data many times larger than memory on one node}
  project/semantics/covers/constraints: {title: Constraints as queries that must return nothing on the new state, reason: every layer the spec makes normative}
  project/semantics/covers/evolution: {title: 'Schema operators, their inverses and version translation', reason: every layer the spec makes normative}
  project/semantics/covers/isolation: {title: Serializability as acyclic dependency graphs over constructor relations, reason: every layer the spec makes normative}
  project/semantics/covers/queries: {title: 'Queries, including match, grouping, fixpoints and nested results', reason: every layer the spec makes normative}
  project/semantics/covers/writes: {title: Change sets and their application as one multiple assignment, reason: every layer the spec makes normative}
  project/semantics/denotational: {title: 'A denotational semantics on paper, over sets as NRC', reason: a set semantics over a kernel calculus on paper now; mechanising waits until the language stops moving}
  project/semantics/kernel/core: {title: A small kernel calculus; the surface is defined by desugaring to it, reason: a small kernel; the surface is defined by desugaring}
  project/spec/conformance/normative-suite: {title: The official suite is normative; passing it is part of conforming, reason: the suite wins when prose and test disagree}
  project/spec/errors/closed-sum: {title: 'Errors are values of closed prelude sums, matched exhaustively', reason: errors are closed prelude sums; clients match named sets}
  project/spec/errors/new-kind/coarse: {title: 'Few constructors by client action, detail as a field', reason: 'few constructors by client action, versioned with the edition a bundle pins'}
  project/spec/errors/new-kind/pinned: {title: Error sums are versioned with the spec; bundles pin the version they match, reason: 'few constructors by client action, versioned with the edition a bundle pins'}
  project/spec/normative/catalog: {title: The catalog relations and their types, reason: everything a client or user depends on}
  project/spec/normative/embed-api: {title: The in-process API, reason: everything a client or user depends on}
  project/spec/normative/evolution: {title: 'Schema operators, inverses and reader pinning', reason: everything a client or user depends on}
  project/spec/normative/language: {title: 'Syntax, typing including exhaustiveness, and query meaning', reason: everything a client or user depends on}
  project/spec/normative/model: {title: 'Types, sums, relation variables, keys and references', reason: everything a client or user depends on}
  project/spec/normative/transactions: {title: 'Serializability, durability and the retry contract', reason: everything a client or user depends on}
  project/spec/normative/wire: {title: The binary wire protocol, reason: everything a client or user depends on}
  project/spec/normative/writes: {title: 'Change sets, constraint checking and retag', reason: everything a client or user depends on}
  project/spec/rationale/design-space: {title: The quandary space is the record; the spec states only rules, reason: the quandary space is the record; the spec states rules}
  project/spec/shape/executable: {title: 'Prose, a reference interpreter and an official test suite, released together', reason: 'prose, reference interpreter and suite released together'}
  project/spec/test-format/script: {title: 'A script of statements and queries with expected rows, as sqllogictest', reason: statements and queries with expected rows}
  project/spec/versioning/editions: {title: 'Dated editions, each replacing the last', reason: dated editions; bundles name theirs}
  project/testing/ctor-partition: {title: A query over a sum equals the union of the same query per constructor, reason: the reference interpreter checks results; partitioning by constructor and by predicate checks the planner without one}
  project/testing/differential: {title: 'Random queries and databases, engine checked against the reference', reason: the reference interpreter checks results; partitioning by constructor and by predicate checks the planner without one}
  project/testing/elle/row-per-element: {title: Insert one row per appended element and read the set, reason: one row per appended element keeps reads traceable}
  project/testing/front-fuzz: {title: 'Coverage-guided fuzzing of parser, type checker and wire decoder', reason: the reference interpreter checks results; partitioning by constructor and by predicate checks the planner without one}
  project/testing/gen/corpus-mutation: {title: Mutate a corpus of queries and keep those that still type-check, reason: untyped generation dies on exhaustiveness; schema walks exercise evolution between writes}
  project/testing/gen/schema-walk: {title: Random sequences of schema operators interleaved with writes and queries, reason: untyped generation dies on exhaustiveness; schema walks exercise evolution between writes}
  project/testing/gen/type-directed: {title: 'Well-typed schemas, data and queries generated from the type system', reason: untyped generation dies on exhaustiveness; schema walks exercise evolution between writes}
  project/testing/isolation-check: {title: Histories of concurrent transactions checked for Adya anomalies, reason: the reference interpreter checks results; partitioning by constructor and by predicate checks the planner without one}
  project/testing/memory-model/loom: {title: Exhaustive interleavings of small latch tests under the C11 model, reason: 'Loom for small latch tests, Miri for UB in the simulated engine'}
  project/testing/memory-model/miri: {title: Run the simulated engine under Miri for UB and data races, reason: 'Loom for small latch tests, Miri for UB in the simulated engine'}
  project/testing/plan-toggle: {title: Every query gives the same result with each rewrite disabled, reason: the reference interpreter checks results; partitioning by constructor and by predicate checks the planner without one}
  project/testing/smo-laws: {title: Property tests of schema operator inverses and version translation, reason: the reference interpreter checks results; partitioning by constructor and by predicate checks the planner without one}
  project/testing/tlp: {title: A query equals the union of its restrictions to p and not p, reason: the reference interpreter checks results; partitioning by constructor and by predicate checks the planner without one}
  project/testing/where/in-sim: {title: Simulated clients inside the deterministic simulator, reason: 'in simulation for replay, real processes for what simulation does not model'}
  project/testing/where/real-process: {title: 'Real server processes killed, paused and restarted, Jepsen style', reason: 'in simulation for replay, real processes for what simulation does not model'}
  project/workload/embedded-oltp: {title: 'Application database inside one process, as SQLite', reason: 'embedded, competing with SQLite today; the niche may later scale up to an indirect competitor to Postgres'}
  query/agg-scope/whole: {title: 'Aggregates apply only outside the match, over its result', reason: arms stay row-level so match composes; per-constructor aggregates are group by ctor}
  query/compile-time: {title: Compile-time check against the declared constructors, reason: a non-exhaustive match is a type error}
  query/construct: {title: A match construct over constructors in the query language, reason: match is sugar over ctor-relations that adds exhaustiveness}
  query/ctor-relations: {title: Each constructor exposed as its own relation, reason: 'primitive: each constructor is a relation and the sum their disjoint union'}
  query/native: {title: A native match operator in the planner, reason: 'one probe of the union index, routing each entry by constructor; union-all stays the semantics'}
  query/optional-sum/null-ctor: {title: 'No NULL, optionality is a constructor of the sum', reason: NULL is a hidden constructor; make it explicit}
  query/tooling/live: {title: Live with SQL tools seeing no sums, reason: 'no SQL, so there are no SQL-only tools to serve'}
  query/union-all: {title: UNION ALL of one select per arm, reason: the semantics of match is a disjoint union of arm queries}
  repl/backup/base-plus-log: {title: A fuzzy page copy of the file plus the log from the copy's start position, reason: 'base copy plus log, incremental by page LSN since the database is one file'}
  repl/backup/incremental: {title: 'Only pages changed since a previous backup, found by page LSN or log summaries', reason: 'base copy plus log, incremental by page LSN since the database is one file'}
  repl/cdc/change-stream: {title: 'The committed change sets of chosen constructor relations, in commit order', reason: consumers are registered like bundles and read the change-set stream}
  repl/cdc/registered-consumer: {title: 'A consumer is a registered bundle, so readers-first gates on it', reason: consumers are registered like bundles and read the change-set stream}
  repl/cdc/retag/ctor-pair: {title: 'A delete from the old constructor and an insert into the new, in one change set', reason: 'a retag is a delete from one constructor relation and an insert into another, as the change set says'}
  repl/cdc/retention/bounded: {title: A consumer position older than a retention bound is invalidated, reason: retention is bounded; a consumer that falls behind takes a fresh snapshot}
  repl/cdc/retention/resnapshot: {title: An invalidated consumer restarts from an exported snapshot and the position after it, reason: retention is bounded; a consumer that falls behind takes a fresh snapshot}
  repl/commit-ack/local: {title: Durable in the local log; replicas follow asynchronously, reason: durability level chosen per transaction}
  repl/commit-ack/quorum: {title: Durable in the logs of a quorum of nodes, reason: durability level chosen per transaction}
  repl/commit-ack/rpo/stated: {title: The spec states the loss window as commits past the promoted replica's applied position, reason: the spec states the loss window of each commit level}
  repl/distribution/never: {title: Never partition one database; scale by replicas and many databases, reason: scale by replicas and by many databases; sharding is not a direction}
  repl/embedded/embedded-replica: {title: 'An application process holds a local replica of a server primary, writes forwarded', reason: Litestream-style log shipping and embedded replicas that forward writes}
  repl/embedded/local-writes/forward: {title: Every read-write transaction runs on the primary, reason: writes at an embedded replica forward to the primary}
  repl/embedded/ship-log: {title: The library ships its log and snapshots to object storage, reason: Litestream-style log shipping and embedded replicas that forward writes}
  repl/failover/manual: {title: An operator promotes a replica; nothing is automatic, reason: 'the spec promises manual promotion with fencing; consensus is a later, separate design'}
  repl/failover/split-brain/epoch: {title: Every log record carries a primary epoch; a node seeing a higher epoch stops and rewinds, reason: promotion bumps an epoch and fences the old primary}
  repl/failover/split-brain/fence: {title: Promotion first cuts the old primary off from its storage or network, reason: promotion bumps an epoch and fences the old primary}
  repl/pitr/log-archive: {title: 'Continuous log archive; restore replays a base to a position, time or named point', reason: restore to any log position or schema version from an archive}
  repl/pitr/op-log-point: {title: A schema operator is a named restore point, reason: restore to any log position or schema version from an archive}
  repl/schema/forward-leases: {title: A replica forwards bundle lease renewals and connections to the primary, reason: schema operators travel in the log; leases renewed at a replica are forwarded to the primary}
  repl/schema/in-log: {title: Operators and stage transitions are log records applied in order with the data, reason: schema operators travel in the log; leases renewed at a replica are forwarded to the primary}
  repl/session/bookmark: {title: Commit returns a log position; a replica read given it waits until a safe snapshot covers it, reason: a commit returns a log position; a replica read given it waits until applied}
  repl/standby-reads/lag/force-point: {title: The primary periodically delays new read-write starts until a safe point forms, reason: the primary emits safe points periodically; replicas report their age}
  repl/standby-reads/lag/report-age: {title: Every replica read reports the commit position and age of its snapshot, reason: the primary emits safe points periodically; replicas report their age}
  repl/standby-reads/safe-snapshot: {title: The primary marks safe snapshots in the log; replica readers run only on them, reason: replicas read only at primary-marked safe points; a prefix of SSI commits can show a state no serial order has}
  repl/unit/physical: {title: 'The redo log itself, page records and full-page images', reason: 'replicas are byte copies with the primary transaction ids, so safe-snapshot markers and page repair work; change sets serve CDC'}
  repl/unit/replica-vacuum/feedback: {title: 'Replicas report their oldest snapshot, per constructor heap, into the primary''s watermark', reason: 'replicas report their oldest snapshot per constructor heap, so replay never cancels a reader'}
  repl/upgrade/replicas-first: {title: A new engine replays the old log format; replicas upgrade before the primary, reason: 'replicas upgrade before the primary, which read-old makes safe'}
  repl/verify/manifest: {title: 'A manifest of page and segment checksums, checked without restoring', reason: every backup carries a manifest and is proven by a test restore that rechecks every invariant}
  repl/verify/restore-check: {title: 'Restore, replay, then re-check every sum invariant over the result', reason: every backup carries a manifest and is proven by a test restore that rechecks every invariant}
  security/assert/asserted: {title: An authenticated application role asserts an end-user principal per transaction, reason: 'the connection''s role, or an end-user principal asserted per transaction by an application role'}
  security/assert/connection: {title: The role the connection authenticated as, reason: 'the connection''s role, or an end-user principal asserted per transaction by an application role'}
  security/audit-store/relation: {title: An append-only system relation whose type is a sum of event kinds, reason: 'an append-only system relation of an event sum, written in its own system transaction so denied calls are recorded'}
  security/audit/auth: {title: 'Authentication, grants, policy changes and bundle registration and retirement', reason: 'a registered call names caller, bundle, query and arguments, so audit is cheap and complete for application access'}
  security/audit/calls: {title: 'Every registered query call with caller, bundle, query and parameters', reason: 'a registered call names caller, bundle, query and arguments, so audit is cheap and complete for application access'}
  security/audit/ops: {title: 'Every schema operator appended to the log, with its author', reason: 'a registered call names caller, bundle, query and arguments, so audit is cheap and complete for application access'}
  security/audit/writes: {title: Every committed change set with its transaction's principal, reason: 'a registered call names caller, bundle, query and arguments, so audit is cheap and complete for application access'}
  security/backup-crypt/as-stored: {title: Backups copy encrypted pages and log as stored; restore needs the key versions they use, reason: backups copy encrypted pages; restore needs their key versions}
  security/catalog/open: {title: Every role reads the whole catalog, reason: applications read the catalog only through registered queries; the ad hoc role is granted catalog constructor sets like any others}
  security/encrypt/io-cost/staging: {title: Encrypt into a per-I/O staging buffer on write; decrypt in place in the frame on read, reason: 'encrypt into a staging buffer on write, so optimistic readers never see ciphertext'}
  security/encrypt/page: {title: The pager encrypts each page as it is written and decrypts it as it is read, reason: 'the pager owns every page write, so it encrypts there; the host cannot see engine-managed direct I/O'}
  security/fields/field-list: {title: A grant lists the fields of each constructor it covers, reason: a grant lists the fields of each constructor it covers}
  security/grain/ctor-set: {title: 'Any constructor set of a relation, the same sets indexes and FKs range over', reason: grants range over the same constructor sets as indexes and references}
  security/grain/partial/refuse: {title: A query reading an ungranted constructor is refused; the role matches over its granted set, reason: 'a role matches over the constructor set it holds, which is exhaustive there'}
  security/ifc/none: {title: No labels; access control only, reason: access control only; registered queries already bound what flows where}
  security/integrity/aead: {title: An authenticated cipher's tag replaces the checksum; a failed tag is a corrupt or forged page, reason: the tag replaces the checksum; a failed tag is a corrupt or forged page}
  security/kek/host: {title: Bytes passed in by the embedding application, reason: each key holder has its source}
  security/kek/kms: {title: Fetched or unwrapped by an external KMS, reason: each key holder has its source}
  security/kek/passphrase: {title: Derived from a passphrase by a slow KDF, reason: each key holder has its source}
  security/keys/wrapped: {title: A key-encryption key wraps data keys; pages name their data key version and rotate lazily as rewritten, reason: pages name their data key version and rotate as rewritten}
  security/leak/hidden-stats: {title: 'Statistics, plans and conflict details are shown only to roles that may read every row they cover', reason: grants stop at constructor sets and applications read through definer queries; what remains is errors and statistics on rows the role cannot read}
  security/leak/opaque-errors: {title: A constraint violation on a row the writer cannot read reports no key or constructor, reason: grants stop at constructor sets and applications read through definer queries; what remains is errors and statistics on rows the role cannot read}
  security/nonce/random: {title: 'A fresh random nonce per page write, stored in the page', reason: 'kill bits and hint bits rewrite a page without a new LSN, so LSN nonces would repeat; a random 192-bit nonce per write, XChaCha20-Poly1305'}
  security/policy/in-queries: {title: 'In registered queries; there are no policies, only what each query selects', reason: 'application code runs only registered queries, so the query is the policy; predicates on relations would add a second meaning to every read'}
  security/principal/sum: {title: A schema-declared sum Principal the engine fills from the session, reason: 'Principal is User, Service or Anonymous, so no comparison silently fails'}
  security/register-priv/holds-reads: {title: 'The registrant must hold, with grant option, every relation grant the bundle uses', reason: 'registering under definer rights grants its reads, so the registrant must hold them with grant option'}
  security/retire-priv/owner-or-admin: {title: The bundle's owner or an administrator, reason: 'retiring stops clients, so it belongs to whoever deploys them'}
  security/rights/definer: {title: The bundle's rights; the caller needs only the call grant, reason: the caller needs only the call grant; the bundle's rights reach the rows}
  security/rights/deputy/caller-bound: {title: The engine passes the authenticated caller as an implicit parameter no client can set, reason: the authenticated principal is an implicit parameter no client sets}
  security/role-drop-peers/remove: {title: Delete mappings to the role in the same atomic role drop, reason: Deleting mappings with the removed role prevents old external credentials from inheriting a recreated identity. It keeps the existing catalog representation and makes renewed trust an explicit administrative act; intentional role replacement pays the small cost of reprovisioning its mappings.}
  security/schema-priv/owner: {title: One schema-owner role, reason: one schema-owner role appends operators}
  security/subject/queries: {title: Grants on registered queries and bundles; calling one is the privilege, reason: applications are granted registered queries; the ad hoc role is granted constructor sets}
  security/subject/relations: {title: 'Grants on relation variables and constructor sets, checked against what a query reads', reason: applications are granted registered queries; the ad hoc role is granted constructor sets}
  security/tamper/ledger: {title: Every commit's change set and audit records are hashed into blocks chained by hash, reason: commit change sets and audit records hashed into chained blocks; the head digest is exported}
  security/tenancy/file-each: {title: Each tenant is its own database file, reason: 'a database file per tenant isolates reads, keys, conflicts and resources with no policy machinery'}
  security/tenancy/fleet/fleet: {title: A tool applies the operator log to each tenant and pins bundles per tenant, reason: a tool applies the log to each tenant file and pins bundles per tenant}
  security/transition/transition: {title: Retag privileges name source and target constructor sets, reason: retag privileges name source and target constructor sets}
  spec/adhoc-failure/ends-text: {title: The first failing item ends the text; nothing commits; later items do not run; the error names its ordinal; inside an interactive transaction none of the text applies and the transaction stays open, reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/adhoc-path/adhoc-write: {title: Ad hoc text may write for principals holding adhoc plus the write privileges, reason: loads and staging repairs need a path; constraints hold and every change set is audited}
  spec/adhoc/names/every-module: {title: 'Ad hoc text belongs to no module; an unqualified name resolves in every user module, then the prelude', reason: 'only an ambiguous name needs qualifying, and no session state is added'}
  spec/after/cursor-fields/name-order: {title: The fields correspond to the ordering keys in the code-point order of their names, reason: 'a record type has no written order, and most ordering keys have no name to borrow'}
  spec/agg-params/one: {title: Exactly one; several values pass as a record; a declaration with more is refused, reason: an aggregate takes exactly one parameter; several values pass as a record; a declaration with more is refused}
  spec/agg-range/any-set: {title: Any set expression; sum and avg refuse a generator over a select-built set, reason: count distinct must be expressible; only sum and avg lose meaning when duplicates vanish}
  spec/anon-sum/one-ctor/one-ctor: {title: 'C { ... } alone is a one-constructor anonymous sum', reason: 'a record is already a one-constructor sum, so a lone constructor needs no second one'}
  spec/anon-sum/prelude-head/type-app: {title: 'A prelude type name followed by { begins a type application, and no anonymous constructor takes a prelude type name', reason: 'a prelude type name always means the prelude type, so Set { a: I64 } is the set of records it prints as'}
  spec/anon-sum/row-types/row-types: {title: An anonymous sum has row types T.C and T.(C | D) with the subtyping and joins of a declared sum, reason: 'without row types, q.C and a match binder over a query result would have no type'}
  spec/audit-relation/seq: {title: 'relation catalog::audit : AuditEvent key seq', reason: 'catalog::audit is declared relation audit : AuditEvent key seq'}
  spec/bad-input-source/empty: {title: Defined only for a statement over one source; otherwise source is "" and line is 0, reason: only a one-source statement has one failing record to name}
  spec/blocked-migration/refuse: {title: Refused at once with a report naming the blockers; BlockingMigration notices go to blocking sessions for BlockingNoticePeriod (24h), reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/bundle-families/family: {title: Bundle names are FAMILY-VERSION; call grants and plan pins carry within a family for equal signatures or checked forms, reason: a new release otherwise needs fresh grants and loses its plan pins}
  spec/bytes-form/hex: {title: 0x followed by two lowercase hex digits per byte, reason: 'a byte reads off the text, and the empty value is not an empty cell'}
  spec/catalog-grants/deployment: {title: 'Deployment state, made by ad hoc statements', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/catalog-key-values/declared: {title: 'The relation''s declared key type: I64 for a catalog id, Bytes for catalog::wasm_modules', reason: '.key(n) of a catalog or sys relation has that relation''s declared key type: Bytes for catalog::wasm_modules, I64 where the key is a catalog id'}
  spec/catalog-refs/key-value: {title: 'They hold the object''s key value, an I64 catalog id, not a Key; the engine keeps their integrity', reason: 'the model rule stays absolute; those fields hold the target''s key value, not a Key; the engine keeps their integrity, and a reader joins on them'}
  spec/catalog/physical/per-ctor-relation: {title: Keyed by constructor relation; fields naming physical detail are Text with implementation-defined content, reason: heap-keyed rows with typed physical fields would make the storage layout a promise}
  spec/collation-fallback/root: {title: 'Root order, as CLDR falls back; sys::collations lists root and the tailored locales', reason: CLDR falls back to root for untailored locales}
  spec/collation-names/cldr: {title: 'The CLDR release of the tailoring data: de@cldr-49', reason: 'the CLDR data fixes the order, whatever library computes it'}
  spec/compression/per-call-off: {title: 'Per call, off by default', reason: compressing attacker and secret data in one TLS frame is a compression oracle}
  spec/conditional/if: {title: 'if c then a else b, sugar for a match on Bool', reason: if and else were reserved with no form}
  spec/constraint-faults/accepted: {title: Partial bodies are accepted; a fault while checking fails the statement, reason: a fault while checking already fails the statement}
  spec/ctor-ids/log-order: {title: Deterministically in log order; shared by instantiations; heaps keyed by relation and constructor; prelude ids fixed below 1024, reason: 'codegen must compute the same ids the database uses, and drivers must decode prelude sums'}
  spec/ctor/type-args/expected-then-fields: {title: Those fixed by the expected type and the fields' synthesized types; an unfixed one is an error, reason: 'Some { value: 1 } then needs neither an annotation nor written type arguments'}
  spec/default-field-rows/per-field: {title: 'One row per field the default fills, each with the clause''s text', reason: 'each field''s default is found by its own id, and changing one constructor''s field touches only its row'}
  spec/defaults-home/relation: {title: As relation clauses applied by inserts and retags into that relation, reason: construction stays pure and one type may serve several relation variables}
  spec/disable/disable: {title: 'A `disable r.C` operator, enable''s inverse, refused while rows of r.C exist or a live bundle can write into it', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/dual-header/two-copies: {title: Two alternating copies on pages 0 and 1 with sequence and checksum, reason: 'a torn header would lose the checkpoint and, for encrypted files, every key'}
  spec/dump-damaged/offset: {title: 'DumpDamaged { offset: I64 }, the byte offset of the first part that fails to authenticate or decode', reason: 'a restore that cannot authenticate or decode a dump fails with DumpDamaged { offset: I64 }, the byte offset of the first part that fails'}
  spec/dump-key/passphrase: {title: 'A Text passphrase, turned into the key by Argon2id with a salt and parameters the manifest records', reason: mirrors the passphrase KEK; a restore needs only the passphrase}
  spec/dump-scope/all: {title: 'Roles, grants, credential verifiers, settings, audit and code bytes besides schema and rows', reason: a restore must keep clients working and the version hash must cover code}
  spec/dump-wrong-key/as-damaged: {title: The same failure as a damaged dump, reason: authenticated encryption cannot tell the two apart}
  spec/duration-form/largest-unit: {title: A DURATION literal in the largest unit that divides it exactly; zero is 0s, reason: 'short, exact, and read back by the literal grammar'}
  spec/embedded-open/register-first: {title: 'Register, supersede earlier bundles, bind host functions, migrate, plan', reason: old bundles otherwise block shipped enables for a lease and downgrades break re-upgrades}
  spec/evaluation/by-value: {title: 'Call by value, left to right within a binding; inlining only of total arguments', reason: faults stay predictable and match the surface text}
  spec/evolution/arm-target/scrutinee-set: {title: Every match in the named object whose scrutinee's constructor set contains the new constructor at the new version; exactly one, reason: matching by the scrutinee's type would count a list that never gains the constructor and miss a set that does}
  spec/evolution/bwd-validity/any-state: {title: 'bwd yields states, not necessarily valid ones', reason: 'after a constraint is dropped, rows violating it map back unchanged'}
  spec/evolution/concurrent-apply/refuse: {title: Refused with MigrationInProgress, reason: a waiting apply could hang a deploy for hours with no report}
  spec/evolution/ctor-order/placed: {title: add constructor appends last unless written before C or after C; split products take the source's position, reason: an ordered sum can gain a stage in the middle without a merge and split}
  spec/evolution/deprecated-value/same-path: {title: Only when an equal value is at the same field path of the pre-state row with the same key, reason: 'a row can still be updated while it holds its deprecated value, yet no new value of a closed constructor enters'}
  spec/evolution/enable-inverse/prior-state: {title: 'The operator that restores the prior state: disable from declared, deprecate from deprecated', reason: 'reverting a re-enable then leaves the constructor deprecated, as it was, not declared'}
  spec/evolution/field-action/field-type-and-alter: {title: 'add field takes a field type, on delete included; alter field changes the action of an existing reference field', reason: 'a field type carries its delete action wherever it is declared, and an action is a property worth changing without losing values'}
  spec/evolution/forward-refs/later-key: {title: Key r in a type may name a relation a later operator of the same migration adds; every other name types against the operators before it, reason: 'a reference cycle must be declarable, and only a reference needs to see ahead'}
  spec/evolution/forward-sets/same-migration: {title: Key r.C and Key r.(Set) may name a relation added later when its type is added in the same migration; constructor sets resolve when the migration ends, reason: 'a printed schema must replay as a first migration, and a sum must reference its own constructors in the operator that adds it'}
  spec/evolution/migration-gap/whole-set: {title: The whole set is refused before any migration applies, reason: 'a wrong set would otherwise leave a deploy half applied, at a version nobody tested'}
  spec/evolution/migration-statement/one-statement: {title: 'One statement: the composite of its operators'' forward maps; constraints of the final schema checked once', reason: 'a script can repair, in the same migration, the data an earlier operator''s new constraint needs'}
  spec/evolution/new-ids/listed-order: {title: 'In listed order, after the operator''s other created objects; every product and re-added constructor takes a new id', reason: two tools replaying the log would otherwise assign different ids and disagree on wire tags}
  spec/evolution/object-ids/every-object: {title: 'Every schema object, computed fields, defaults and stored views included', reason: an object named by path would change name on a rename and break inverses by id}
  spec/evolution/operator-type/catalog-sum: {title: 'A catalog sum Operator, one constructor per operator, objects by catalog id; operators are rows keyed by (migration, ordinal), hashed in the wire value encoding', reason: every implementation and tool can then recompute a version's hash from a stated value}
  spec/evolution/preconditions/predicate-read: {title: Every precondition is a predicate read of the migration's transaction, reason: a precondition tested only on the snapshot lets a concurrent insert leave a row of a removed constructor}
  spec/evolution/product-position/first-source: {title: A merge or nest product takes its first source's position; unnest products take the source's; a re-added constructor its position before removal, reason: a revert of remove then puts the constructor back where it was in an ordered sum}
  spec/evolution/product-state/source-state: {title: 'The source''s state, per relation variable', reason: a split of a deprecated constructor would otherwise reopen writes the author meant to retire}
  spec/evolution/replica-schema/safe-snapshot: {title: 'Replicas serve consistent safe snapshots, so no row of a constructor appears before its declaration', reason: an apply-order rule is a procedure and does not by itself give a reader a consistent view}
  spec/evolution/revert-fill/per-field: {title: 'One fill per field, fill T.C.f = e, with the form and scope of add field''s fill', reason: one fill for the whole revert cannot give different fields different values}
  spec/evolution/revert-identity/by-id: {title: 'Inverses computed by object id before the operator, resolved to current names at apply; fill clauses; dependants by object id', reason: 'a rename or a reused name cannot misdirect the inverse, and a drop field stays revertible once its fill is written'}
  spec/evolution/split-inverse/every-product: {title: Invertible only when every product has an arm; a product no arm reaches makes the split lossy and its revert is refused naming it, reason: an inverse merge needs an arm for every product}
  spec/evolution/split-self-ref/product-set: {title: 'A product field naming the split constructor is retyped to the products'' set, as every reference to it is, unless the split names the target; the routing types a carried field at its retyped type', reason: 'a reference to the split constructor is retyped the same way wherever it is written, and a carried value keeps its key'}
  spec/evolution/staged-meaning/one-transaction: {title: 'One transaction serialized at its final commit, plus the progress row and the failure report', reason: stating each intermediate state would fix one build strategy and expose states no query can name}
  spec/evolution/term-encoding/canonical-tokens: {title: 'Text: the resolved term''s tokens in canonical spelling, joined by one space, comments dropped', reason: a syntax tree encoding would need a second schema for the whole grammar to feed one hash}
  spec/evolution/translatable/semantic-via-triggers: {title: 'The semantic criterion, stated through listed syntactic triggers with a defined read', reason: 'the list alone would accept cases that fail at call time, and the criterion alone is undecidable'}
  spec/evolution/translated-fields/same-ctor: {title: 'From the existing row only when it has the new value''s constructor; else the current default, else the fill', reason: 'a field of another constructor may mean something else, and a translated insert matches the same insert at the current version'}
  spec/evolution/version-hash/value-encoding: {title: 'The wire value encoding of the resolved operator list, as values of the catalog''s operator type', reason: fixing a printing bug would otherwise break every version check}
  spec/exact-sum/exact: {title: 'The exact total, checked once at the end', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/examples/mixed-blocks/split: {title: Split it into an adt block and a text block, reason: 'a text block leaves its adt lines unchecked, and skip marks would be one more syntax'}
  spec/examples/self-contained/in-block: {title: Each block declares or restates every temp relation and type it uses beyond the running schema, reason: each block declares or restates what it needs; the corpus harness keeps no table of continuations}
  spec/family-owner/first-registrant: {title: The first registrant owns the family; only it or the schema owner registers into it; family call grants compare signatures by type ids, reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/fault-type/fault-sum: {title: 'A prelude Fault sum; try catches faults, not fail', reason: try needs a result type}
  spec/field-faults/any: {title: Any fault some field raises; the fields of one record or constructor value are not ordered, reason: which fault a value reports is no more fixed than which binding faults}
  spec/fix/bounded/element-flow: {title: Only where its result can become an element of a relation of the stratum, reason: a value only compared never enters the fixpoint}
  spec/fix/projection/not-constructing: {title: 'Not constructing: start, end, unwrap and value return a component of an argument', reason: a component of an argument is drawn from values already present}
  spec/fix/restriction/positive: {title: 'Positive: if e ▷ S is positive, so is e', reason: 'restriction keeps a subset of its operand, so it is monotone'}
  spec/fix/typing/least: {title: Each binder's element type is the least type at which every body checks against Set of it, reason: 'it fixes which programs are accepted without an annotation, a bottom type or one inference procedure'}
  spec/fn-results/allowed: {title: 'Yes; function types never appear in a relation, field, set element, type argument, query result or registered-query parameter; all normalised away before planning', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/fpi/clean-to-dirty: {title: 'On each clean-to-dirty transition, skipped when page writes are atomic', reason: 'fuzzy checkpoints start redo before the checkpoint, so per-checkpoint images can miss a torn page'}
  spec/grant-carry/carry: {title: Rename and nest carry; split grants each new constructor the kept fields; merge requires equal source grants; remove drops, reason: schema changes must not silently revoke or widen access}
  spec/grant-kinds/split: {title: 'Relation, constructor-set and call fn grants are migration operators; roles, capabilities and bundle call grants are ad hoc statements; each refused in the other place', reason: access to data shapes travels with the schema and its hash; who may deploy and operate is deployment state}
  spec/group/binder-type/binder-only: {title: 'Group K T is only the type of a group binder; passed on, a group is its nonempty Set T', reason: 'a function then receives an ordinary set, and no aggregate ranges over a caller''s group'}
  spec/header-24/slot: {title: In the slot; the version header stays 24 bytes, reason: keeps the ruled 24-byte header with 64-bit tids}
  spec/idempotency/recorded: {title: 'Optional, increasing per client; committed replies recorded with rows; low-water mark; RequestExpired', reason: a purged record cannot be told from a new request without increasing ids}
  spec/ids/engine-objects/fixed-table: {title: 'catalog.md §4.6 lists them per id space, below 1024, per catalog version', reason: the engine's types already take a fixed table; fixed ids per catalog version stay stable when the engine's declarations change}
  spec/ids/move-down/first-declared: {title: The listed constructor first in the type's declaration order; the others take new ids in that order, reason: a move down keeps the id on the first listed constructor in the type's declaration order}
  spec/ids/moved-field/keeps: {title: The moved field keeps its id; a move down gives each further constructor a new field, reason: 'a move keeps the field''s data, as a rename does, so it keeps its identity; a move down gives each further constructor a new field'}
  spec/ids/operator-ids/implied: {title: Only the first id is recorded; the rest follow from §1.2's order and the decl term, reason: the ids follow from the order of catalog.md 1.2 and the decl term; recording them again could only disagree}
  spec/ids/package/as-operators: {title: 'As if the package''s declarations were operators of the installing migration, after install, in package order', reason: one id space and no reuse follow from treating the package's declarations as operators of the installing migration}
  spec/ids/prelude-fns/none: {title: 'No: the prelude is not part of a schema, so its functions have no id and no row', reason: 'prelude functions have no catalog id and no row: the prelude is not part of a schema'}
  spec/ids/rescope/named: {title: 'Split, merge, nest and unnest re-scope an index whose clause names a replaced constructor; one with no in keeps its id', reason: 'unnest re-scopes indexes as split, merge and nest do; an index with no in is not re-scoped and keeps its id'}
  spec/ids/rescoped-index/new: {title: 'It is a new index with a new id, created after the operator''s constructors; the old one is dropped', reason: 'a re-scoped index may merge several into one, so no single id can survive; a new index with a new id is unambiguous'}
  spec/ids/view-type/anonymous-only: {title: 'store f creates a one-constructor type, after the view''s relation variable, only when the view''s element type is anonymous; a view of a declared type is a relation of that type', reason: store f creates a wrapper type only when the view's element type is anonymous; a view of a nominal type is a relation of that type}
  spec/index/kinds/declared-capabilities: {title: The index forms a schema declares and what each lets a query do; not the structure behind them, reason: promising ordered trees would rule out hash indexes and tie the spec to one structure}
  spec/inline-types/named-only: {title: Every sum or record used as a field type is a declared type, reason: nominal typing holds without exception and evolution has a name to act on}
  spec/join/no-expected/least: {title: The least type T such that every arm checks against T; none is an error, reason: 'if c then None else Some { value: 1 } types without an annotation'}
  spec/key-field/declared: {title: The type declares the key fields; a self-key field is typed Key r and binds the type to r, reason: an added field cannot belong to a type that serves several relation variables}
  spec/key-preserving/params-and-groups: {title: Also the binder of a generator over a Set T parameter or over a group's rows, reason: 'such a binder ranges over distinct elements, so its select merges nothing'}
  spec/key-too-large/rejected: {title: Reject the statement with KeyTooLarge naming the index and payload limit, reason: A row can fit its heap while an index entry cannot fit. A typed statement rejection identifies the actionable index limit without disguising valid input as corruption; overflow keys would require a separate storage design.}
  spec/key-values/constructor: {title: 'r.key(v) or r.key(f: v, g: w); no literal checks against Key r', reason: 'a key names its relation, so it cannot be mistaken for another relation''s key'}
  spec/lambda/params/curried: {title: 'fn (a: T, b: U) -> e has type T -> U -> R; a call applies its arguments in order', reason: 'a named fn with several parameters can be passed, and arguments stay out of record rules'}
  spec/late-relation-state/type-state: {title: 'The constructor''s state in its type, from catalog::value_states', reason: a relation added after its type gained a constructor takes the type-level write state for that constructor}
  spec/lease-expiry/logged: {title: 'Expiry computed from the last logged renewal on every node; the primary waits a further LeaseGrace (1h); an expired bundle is not live, and binds, calls and renewals are refused BundleExpired until re-registration', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/lease-renewal/calls: {title: 'On binding and on calls, written at most once per lease/8', reason: a long-lived connection must not count as dead while serving calls}
  spec/limit-reason/limit-sum: {title: 'StatementLimit carries limit: Limit = Time { after: Duration } | Memory { bytes: I64 } | Rounds { rounds: I64 }', reason: 'StatementLimit carries limit: Limit, with Limit = Time { after: Duration } | Memory { bytes: I64 } | Rounds { rounds: I64 }; the last is the fixpoint round limit'}
  spec/log-content/authored: {title: 'Only author-written migrations; retirement, leases and supersession are deployment state', reason: engine appends diverge the file from shipped migrations and fail the next open}
  spec/log-framing/batch-framing: {title: Blocks carry batch sequence and block count; incomplete batches discarded; the tail zeroed before new writes, reason: stale blocks of a discarded batch keep valid checksums and could be read as a continuation}
  spec/log-rows/applied-time/wall-clock: {title: 'The wall-clock time the migration was applied, in its transaction', reason: 'the row is written inside the applying transaction, before its commit time exists'}
  spec/log-rows/check-body/whole-decl: {title: 'The whole declaration after the verb, so a check keeps its on target', reason: 'a check''s on target is part of what it checks, so the row and hash keep it'}
  spec/log-rows/fields-term/bare-list: {title: The field list without its parentheses, reason: 'the parentheses belong to the clause, not the list'}
  spec/log-rows/fn-determinism/transitive: {title: 'Only when every function it calls, transitively, is deterministic', reason: 'a wrapper must not launder a nondeterministic call into keys, indexes and constraints'}
  spec/log-rows/fn-text/as-written: {title: As written; an aggregate's body is everything after =, reason: the row shows what the author wrote; the hash already covers the canonical form}
  spec/log-rows/unscoped-members/written-scope: {title: 'Only a written in r.C scope gives Member clauses; an unscoped one has none and ranges over every constructor, later ones included', reason: 'an unscoped index covers constructors added later, so listing today''s would misstate its range'}
  spec/membership/pinned: {title: Membership at the pinned version; enabling C blocks on readers of the whole relation or of references admitting C, reason: old readers keep exhaustive matches and never dereference into an unknown constructor}
  spec/mixed-versions/refuse: {title: Refused with MixedVersions; the transaction stays open, reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/nesting-limit/balanced: {title: '64 levels; and, or and union chains count by balanced tree height', reason: 'and, or and union are associative, so a balanced tree is the same query; generated filters stay well under the limit'}
  spec/newtype-syntax/call: {title: Email("a@b") and type Email = new Text where p, reason: juxtaposition had no production and is ambiguous with nullary constructors}
  spec/only/none: {title: No surface only; a row of the snapshot is reached by a path through its key or by match on the key, reason: 'no example needs it yet, and adding it later changes no program'}
  spec/option-ref-delete/clear: {title: on delete clear sets an Option (Key) field to None, reason: the common case needed a hand-written update in every deleting statement}
  spec/order-keys/list: {title: 'A list of keys with directions: by (a desc, b)', reason: 'anonymous records stay unordered, and field order does not affect type equality'}
  spec/partiality/fault-or-diverge: {title: 'May fault or may not terminate; limits are rejections, never faults', reason: an unbounded fix must not move past a guard although no fault is raised}
  spec/password-param/text: {title: Text, reason: SCRAM normalizes passwords as text}
  spec/past-reads/safe-snapshot: {title: 'The greatest safe snapshot at or before p, listed in sys::safe_snapshots, typed at that snapshot''s schema version; p is a Position; no SIREAD locks', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/pattern/named-set/like-ctor: {title: 'The payload forms of a constructor pattern, over the fields every member shares', reason: 'a named set reads like a constructor in the same match, with no value bound only to be ignored or projected'}
  spec/pattern/redundant-alternative/refused: {title: 'Refused, as an unreachable arm is', reason: a dead alternative is as much a mistake as an unreachable arm}
  spec/peer-mappings/relation: {title: '`catalog::peer_mappings` keyed by (listener, ordinal), changed by `map peer` and `unmap peer` by `schema` holders; one relation for certificate subjects and OS users; `*` matches any run', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/pending-relaxations/all: {title: '`drop constraint`, `drop unique`, `drop check` and a relaxing `rekey`; pending drops are carried and rechecked, and an operator leaving one without an image is refused', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/pinned-defs/own-version: {title: 'The definitions of its version; indexes, constraints and stored views use current code', reason: a pinned bundle's meaning does not change under it; an expression index is used only when code identity matches}
  spec/prelude/prelude-part: {title: 'A normative prelude part with every type and function, signatures and faults', reason: about a dozen functions were used and defined nowhere}
  spec/primary-reads/never: {title: Never; they run under SSI; positions carry the epoch, reason: reads must not wait for writes; read-your-writes must survive failover}
  spec/privileges/fifteen: {title: 'read, insert, update, delete, retag, call, adhoc, assert, schema, monitor, cancel, maintain, configure, read_files, write_files', reason: one list in syntax and security; the key field is granted with any read}
  spec/quarantine/index/cost-only: {title: 'No read fails: queries do not use the index, so only cost changes; a write to a constructor relation the index covers fails', reason: whether a read runs must not depend on which plan the planner picks}
  spec/query-result/any: {title: Any type; a comprehension is a set, reason: a single aggregate result needs no host comprehension}
  spec/reads/static/static-types: {title: The constructors in the types of the relation expressions of the normal form; parameters and pruning do not change it, reason: whether a call fails under quarantine must not depend on the planner or on its arguments}
  spec/record-add-ctor/allowed: {title: 'Allowed, as on any sum: a record is a one-constructor sum; registered queries are rechecked', reason: 'a record is a one-constructor sum; registered queries are rechecked, and the migration is refused if one fails'}
  spec/regex-classes/unicode: {title: 'Unicode: \d, \w, \s and \b follow the recorded Unicode version', reason: 'the classes are Unicode, following the database''s recorded Unicode version, since matches works over Unicode scalar values'}
  spec/regex-flags/re2: {title: 'RE2''s i, m, s and U, anywhere in the pattern; x and R refused; . excludes a line feed unless s', reason: RE2 patterns keep working; x and R are not RE2}
  spec/rejected-source/unqualified: {title: The relation's unqualified name, reason: a rejects relation sits beside the load it serves}
  spec/row-too-large/refuse-at-write: {title: 'Refused at the statement with RowTooLarge naming the relation, constructor, size and limit; a migration declaring a constructor whose fixed part can never fit is refused', reason: 'a row too large fails the one statement that writes it with a reason the caller can act on, and the migration refuses a constructor no row of could ever be stored'}
  spec/schema-fn-calls/import: {title: import fn NAME gives it a bundle ordinal, reason: calls stay ordinal-addressed and checked at registration}
  spec/schema-principal/root-first: {title: 'In the root namespace, resolvable from every module and bundle, declared only in the first migration, with log ids; prelude ids 37-40 belong to the default', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/schema-siread/version-item: {title: Every statement reads the schema version item; a migration writes it, reason: a writer at n-1 could commit into a constructor version n deprecated without any conflict}
  spec/script-format/indented: {title: 'Directives in the first column, bodies indented two spaces, rows after ---- up to a blank line, comment or directive; sets in printed order unless rowsort', reason: 'the layout the conformance.md example already uses, with the ends of bodies and rows and the meaning of query without rowsort stated'}
  spec/scripts-pinning/never: {title: 'Never; a script changes data, so translation across it is the identity', reason: blocking would stop every data-fix migration for a lease}
  spec/settings-change/command: {title: 'set setting X = v, needing configure', reason: 'settings are effects, not data a registered query writes'}
  spec/snapshot-edges/serialization: {title: 'A conflict, wait or key violation against such a row is a serialization failure; no re-evaluation on successors', reason: re-evaluation mixes post-snapshot rows into a snapshot statement}
  spec/span-names/covers: {title: covers for spans; contains for text only, reason: avoids overloading outside conversions}
  spec/staged-failure/discard: {title: The pending migration is discarded and the log is unchanged, reason: a version commits only when its staged jobs complete}
  spec/stale-bundles/refuse: {title: Refuse binding an expired bundle with untranslatable queries and registering one pinned before an enabled constructor it reads, reason: otherwise one stale bundle wedges every later migration or reads rows it cannot type}
  spec/stragglers/hide: {title: 'A bundle may declare hide new constructors: rows of unknown constructors are invisible to it, and following a reference to one faults NoSuchRow', reason: 'one old phone calling forever would hold back a release, and retiring breaks every old client'}
  spec/supersede/command: {title: '`supersede bundle B by B2`, run by B''s owner or the schema owner, B2 live in the same family', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/supersession-commit/before-migrations: {title: 'Durably and on its own, before the shipped migrations; undone only by re-registering; registrations stay atomic', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/take-negative/fault: {title: 'Faults with NegativeCount, as top does', reason: one rule for the counts of take and top}
  spec/temp-relations/generated-key/none: {title: 'No: no type names a temp relation, so its key is plain fields or its whole row', reason: no type names a temp relation}
  spec/temp-relations/names/declared: {title: 'No name a module the text reads declares: relation, function, type or constructor', reason: 'a temp relation may not take any name a module the text reads declares: relation, function, type or constructor'}
  spec/temp-relations/redeclare/refused: {title: 'Refused, as any repeated declaration is', reason: as any repeated declaration is}
  spec/temp-relations/sum-key/declared: {title: It must declare its key; one without a key clause is refused, reason: a temp relation of a sum with several constructors must declare its key}
  spec/temp-relations/txn: {title: 'Temp relations scoped to one interactive transaction or one ad hoc text, outside the log', reason: sessions hold no state; every connection continuing the transaction sees them}
  spec/text-escapes/unicode-escape: {title: 'Printing writes a control character as \u{..}; reading accepts it raw or escaped', reason: printed text holds no invisible characters}
  spec/textual-forms/interop: {title: 'interop.md §1 owns them: NaN, inf, -inf, ISO 8601 times, P{m}M{d}D periods; a query writes them as T.parse calls', reason: 'one part owns the forms, and the lexer gains no tokens for them'}
  spec/textual-forms/sortkey/hex: {title: '0x hex, as Bytes', reason: 'a SortKey is bytes, ordered as bytes'}
  spec/textual-forms/span/call: {title: 'span(a, b), the call that builds it', reason: span is the only form that builds a span}
  spec/tiers/three-tiers: {title: 'Three tiers: docs/spec what users observe, docs/design contracts between subsystems, the rest with the code', reason: annex text in the spec is read as a promise and couples the spec to one implementation}
  spec/top/operand/comprehension: {title: The operand is a comprehension; the keys are written over its select scope, reason: 'the select scope binds the keys, so no new binder form is needed'}
  spec/totality/add-period/partial: {title: 'Partial: it can fault with ArithmeticOverflow', reason: it faults with ArithmeticOverflow under either rule}
  spec/totality/at-block/partial: {title: 'Partial, because HistoryUnavailable is an abort', reason: 'HistoryUnavailable is an abort, and a hoisted block could reject a program that never reads it'}
  spec/totality/top-take/non-literal: {title: Partial unless the count is an integer literal, reason: 'a literal count is never negative, so never faults'}
  spec/try-faults/any: {title: Any one of the faults; a listed freedom; conformance accepts any fault some binding raises, reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/txn/call-atomic/item-statements: {title: 'Each item is a statement; a call is atomic and its failure undoes all its statements, leaving the transaction open', reason: the same text keeps the same constraint boundaries inside and outside an interactive transaction}
  spec/txn/durability/strongest: {title: 'The strongest level named by its begin, its commit or any block run in it; deferrable inside is refused; read_only with durability is a type error', reason: a block that asks for quorum is never silently acknowledged at local}
  spec/txn/isolation-statement/serial-order: {title: As the serial-order property of the committed transactions alone, reason: two implementations that track reads differently still promise the same thing}
  spec/txn/may-abort/may: {title: Any transaction may be aborted; committed ones always have a serial order; read_only deferrable is the exception, reason: every practical detector aborts some transactions that could have committed}
  spec/txn/one-version/one-pair: {title: One schema version and one catalog version for every statement; a call at another is refused with MixedVersions, reason: two bundles pinned at different versions would otherwise mix versions in one transaction}
  spec/txn/real-time/primary: {title: 'On the primary, a transaction acknowledged before another began precedes it; replica reads keep their own guarantee', reason: a client always sees its own acknowledged commit on the primary}
  spec/txn/request-order/increasing: {title: The numbers a client sends with one client id must increase, reason: 'with numbers in any order, a purged record could not be told apart from a new request'}
  spec/txn/resend-unknown/not-committed: {title: A definite not-committed error, reason: a generic unknown-transaction error leaves the client unable to tell whether to rerun}
  spec/txn/resend/promise: {title: A resend after commit returns the original reply and never reapplies, reason: 'the caller observes the reply, never how the engine records it'}
  spec/txn/retry-count/n-plus-one: {title: 'At most n + 1 attempts, the first included; with no option the configured limit applies, in process as on the server', reason: 'retry n counts reruns, so retry 0 still runs once'}
  spec/txn/safe-point/liveness: {title: A liveness promise bounded by SafePointDelay; the delaying mechanism leaves the spec, reason: 'the bound a reader observes is promised outright, not implied by a mechanism'}
  spec/type-args/stop/clause-words: {title: A type argument list stops before a contextual clause word and before a name followed by (, reason: 'clause words stay usable as names, and generic relation types need no parentheses'}
  spec/unicode-version/one: {title: 'The database''s one recorded Unicode version, as every other text function', reason: one recorded Unicode version for every text function}
  spec/unique-names/every-ctor/unscoped: {title: 'As the unique with no in: the same scope, one canonical name', reason: one scope has one canonical name}
  spec/unique-names/named-set/ctors: {title: 'By the set''s constructors, in code-point order: in (Apple | Google)', reason: the name states the constructors the unique ranges over}
  spec/unique-names/unscoped/bare: {title: 'Without an in clause: m::r.unique(p1, ..., pn)', reason: the name is the declaration as written}
  spec/unique-violation/violated: {title: KeyViolation for the key only; any other unique reports Violated with that unique, reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/unit-print/parens: {title: The unit value prints as `()`, reason: 'the literal, the type and every surface mention spell it (), and equal values print identically'}
  spec/upsert-new/old-and-new: {title: Both old and the proposed row new; a single-row let binds exactly one row, reason: a bulk merge must read the incoming row}
  spec/uuid-domain-format/lossless: {title: Use generic newtype text outside the sixteen-byte UUID domain, reason: 'Preserve write-only newtype domains, existing sixteen-byte UUID presentation and total lossless formatting by reusing the generic newtype spelling for other lengths'}
  spec/uuid/uuid-newtype: {title: A prelude Uuid newtype over 16 bytes with uuid() and text(), reason: keys need a readable textual form}
  spec/value-states/gated: {title: '`add constructor` on a value-level sum starts declared; `enable T.C` gates storing it in any field; refused while a live bundle pinned before reads a field whose type reaches T, hide new constructors included; storing a declared one is Rejected NotOpen', reason: maintainer rulings on the third spec review; reasons in each option body}
  spec/violation-report/one: {title: 'The violated constraint with the smallest name, its report as descriptor and rows, unreadable rows redacted with a count', reason: an error value is not a collection; one deterministic constraint suffices to act on}
  spec/write-forms/skip-seq-if: {title: 'skip, parenthesised write sequences and if writes', reason: conditional and multi-part writes otherwise need dummy generators}
  spec/writer-wait-cycle/fail-one: {title: One transaction of the cycle fails with a serialization failure; which one is the engine's choice, reason: 'a cycle of waits fails one of its transactions with a serialization failure, which one being a freedom'}
  spec/writes/abort-choice/any-abort: {title: 'Any one of the aborts, as a declared freedom', reason: 'a block need not run in written order, and write order stays unobservable through errors'}
  spec/writes/action-compose/one-value: {title: 'Declarative: the gone keys close under cascade; each surviving referrer takes one new value from every action reaching it', reason: 'two clears on one row, or a clear and a retag, would otherwise be a KeyViolation the author cannot avoid'}
  spec/writes/action-identical/identical-only: {title: 'Two retag actions reachable on one type must be the same action (same constructor, same fills); otherwise a schema error', reason: 'the outcome would otherwise depend on field order, which means nothing elsewhere in a schema'}
  spec/writes/action-own-writes/statement-wins: {title: Actions reach only pre-state rows the statement neither replaces nor deletes; a replaced row is judged by its new value, reason: repointing a reference and deleting its old target in one statement then succeeds}
  spec/writes/action-reach/pre-state: {title: 'Only pre-state rows; an inserted row referencing a gone key violates its reference, whatever the action', reason: 'every action treats an inserted referrer alike, and no statement silently discards a row it wrote'}
  spec/writes/bound-row/snapshot-row: {title: 'x bound to a snapshot row: a generator, an upsert''s old, a let of a row read from the snapshot, or a match binder over one; never a row a write produced', reason: a row only proposed by the same statement has no pre-state row to update or delete}
  spec/writes/insert-existing/violation: {title: 'An insert never replaces; a pre-state key not deleted by the change set is a KeyViolation, whatever the value', reason: an insert that silently matched an existing row would report unchanged rows as inserted}
  spec/writes/let-write/four-forms: {title: 'insert, upsert, update (new rows, fills applied) and delete (the rows it names); any other write is a type error', reason: 'a sequence over several relations has no row type, and fail and skip bind nothing useful'}
  spec/writes/nested-insert/once: {title: 'Only where evaluated exactly once per statement: under no binder and in no conditional position', reason: 'under a binder or a condition, how many rows it inserts and which key each use denotes would depend on values'}
  spec/writes/new-key/ctor-expr: {title: A constructor expression giving a key field is a type error; KeyChanged is the dynamic failure for every other new value, reason: which programs are accepted would otherwise depend on how clever the checker is}
  spec/writes/rejects-source/one-source: {title: Only an insert whose source is a single generator over a source relation or an external source; otherwise a type error, reason: a row computed from a join or an aggregate has no single source record to report}
  spec/writes/rejects/per-row: {title: 'Each source row''s conversion is tried; rows that convert are inserted, rows that fail go to r with their position and error; load is fresh per statement', reason: otherwise two implementations may disagree on which rows land where}
  spec/writes/retag-literal/nullary: {title: Scalar literals and nullary constructors, reason: a nullary constructor is as constant as a literal}
  spec/writes/sequence-distinct/issued-and-held: {title: A generated key differs from every value issued and from every key any committed row of the relation has held, reason: a counter rule says nothing of a key held only by a row since deleted}
  spec/writes/unique-name/fields-or-id: {title: 'A field unique is named by relation, field paths and constructor scope; an expression unique is r.unique#n, n its catalog id', reason: naming by expression text would need a canonical printing of every expression}
  spec/writes/update-fill/value: {title: 'The new value is a value: an omitted own field takes the relation''s default for its constructor; with x copies every field x has', reason: an omitted field then means the same whatever constructor the row had}
  spec/writes/upsert-binding/change-set: {title: 'The row with old''s key among the else branch''s effects, or old; defined from the change set', reason: 'a nested let, a sequence or a match would otherwise change what is bound without changing the change set'}
  spec/writes/upsert-delete/through-actions: {title: 'Any delete of old the branch can cause, delete actions included', reason: a cascade removes the row as surely as a delete the branch writes}
  spec/writes/upsert-scope/subset: {title: The target's constructor set is a subset of the unique's; an expression unique conflicts on equal expression values, reason: 'a row outside the unique''s scope never conflicts, so its else branch would be silent dead code'}
  spec/writes/violation-name/canonical-least: {title: Every generated constraint has a canonical qualified name; the least name is reported; a fault while checking wins, reason: one violation then reports the same constraint across implementations and runs}
  spec/writes/where-alone/conditional: {title: 'The write on x when p holds, and skip otherwise', reason: 'the grammar admits the form, and the conditional is the only reading it has'}
  spec/zone-edges/overflow: {title: They may fault ArithmeticOverflow, reason: as every other time operation at its range edge}
  spec/zone-names/exact: {title: Matched by exact spelling and kept as written, reason: the zone a client wrote comes back as written}
  spec/zone-overlap/earlier: {title: 'Not in the later half of an overlap: it gives the earlier instant, and the spec says so', reason: the round trip loses the later instant; state it}
  spec/zoned-edges/overflow: {title: 'It may fault ArithmeticOverflow, as in_zone and to_instant do', reason: as in_zone and to_instant do}
  storage/sum-scan/merge-append: {title: Merge-append ordered scans of each heap, reason: a sum scan is the disjoint union it denotes}
  storage/table-per-ctor: {title: One heap per constructor, reason: 'constructors are the primitive relations, so each gets its own heap; adding one is a new empty heap'}
  substrate/new-engine: {title: New database engine}
  sum/closed: {title: 'Closed, the declaration lists every constructor', reason: 'a sum is closed by definition; adding a constructor is schema evolution, not extension'}
  sum/disjoint-total: {title: 'Only disjoint and total, i.e. sum types', reason: 'one concept, the sum; overlap and partiality are relationships to sums'}
  sum/overlap/facets-1n: {title: 'Entity 1:N a sum of facets, unique (entity, tag) when one per kind', reason: 'ordinary schema patterns, no engine mechanism'}
  sum/overlap/facets-optional: {title: 'One optional 1:1 relation per facet', reason: 'ordinary schema patterns, no engine mechanism'}
  sum/relation: {title: 'Relation-level sum, an entity set partitioned into constructors'}
  sum/value: {title: 'Value-level sum, a column whose type is a union'}
  surface/adhoc/scoped: {title: Ad hoc queries only on a separate interactive or admin role, reason: 'ad hoc queries only in a separate role, outside the registered guarantee'}
  surface/audited-write: {title: 'Ad hoc may write, with every change set logged', reason: 'loads, staging repairs and one-off fixes need a path that is not a migration or a bundle per fix; every ad hoc change set is audited with its principal, and constraints hold as always'}
  surface/both: {title: 'A position for humans, a hash for checking', reason: 'bundles pin the hash, people read the position'}
  surface/codegen/no-sums/sealed: {title: Sealed interfaces or a visitor per sum, reason: 'sealed hierarchies where the host has them, a tag plus payload struct otherwise; the engine never sees it'}
  surface/codegen/no-sums/tagged-struct: {title: A tag enum plus one optional field per constructor, reason: 'sealed hierarchies where the host has them, a tag plus payload struct otherwise; the engine never sees it'}
  surface/codegen/query-functions: {title: One function per registered query with exact argument and result types, reason: 'types from the schema, typed functions from registered queries'}
  surface/codegen/schema-types: {title: Host types for every sum and constructor in the schema, reason: 'types from the schema, typed functions from registered queries'}
  surface/constraint/check: {title: Per-constructor checks over the row's own fields, reason: 'row checks as sugar, denials as the general form'}
  surface/constraint/denial: {title: Named queries that must return nothing, reason: 'row checks as sugar, denials as the general form'}
  surface/ctor-set/growth/declare: {title: The add operator names the sets the constructor joins, reason: 'adding a constructor states which named sets it joins, as it states its match arms'}
  surface/ctor-set/list: {title: An explicit list of constructors, reason: 'constructor sets written explicitly or named; no except, which would grow silently'}
  surface/ctor-set/named: {title: Named subsets declared in the schema, reason: 'constructor sets written explicitly or named; no except, which would grow silently'}
  surface/extent/separate: {title: A sum is a type; relations of that type are declared apart, reason: 'follows model/set-of-type: a type and the relation variables holding it are distinct'}
  surface/generated: {title: The engine prints the folded schema; the printout is committed and diffed in review, reason: review sees each operator and the schema it produces}
  surface/introspection/catalog-relations: {title: The catalog is a set of relation-level sums in the same language, reason: follows engine/catalog-relations}
  surface/lease-policy/translate-stale: {title: An expired bundle no longer blocks changes but keeps running through translation, reason: expiry stops a bundle holding back changes; it keeps running through translation}
  surface/migration/handwritten: {title: The developer writes the operators, reason: 'operators are written, never inferred from a diff, so rename and split are never ambiguous'}
  surface/namespace/modules: {title: Named modules with qualified references, reason: schemas and query bundles compose by module}
  surface/open-applies: {title: open() applies bundled migrations and registers the app bundle, reason: 'an embedded file is opened, migrated and registered in one call'}
  surface/query-scoped: {title: Anonymous result constructors are numbered within the registered query signature, reason: 'anonymous result records are structural, so anonymous result sums are too, numbered within the query signature'}
  surface/reader-tracking/bundles: {title: Each client release registers a query bundle tagged with its version, reason: a registered bundle states the schema version it reads}
  surface/reader-tracking/dormant/lease: {title: Bundles expire unless renewed by a connecting client, reason: 'a bundle holds its version by lease, and an operator may retire it'}
  surface/reader-tracking/dormant/retire: {title: 'An operator retires a bundle, whose queries are then refused', reason: 'a bundle holds its version by lease, and an operator may retire it'}
  surface/ref/ref-type: {title: A reference type naming a constructor set, reason: 'Key S names its constructor set, as model/typed-key states'}
  surface/registration/build-time: {title: 'A build step sends query files to the engine, which checks and stores them', reason: client bundles registered at deploy; schema functions live in the catalog}
  surface/registration/on-change/refuse: {title: The migration is refused while any active query fails, reason: a change that breaks a live bundle is refused until the bundle is retired}
  surface/registration/order/pinned: {title: Each bundle is checked against the version it names; declared constructors stay write-disabled, reason: 'a constructor is declared, bundles pin the new version, then writes are enabled'}
  surface/registration/schema-resident: {title: 'Queries are named definitions in the schema, deployed by migrations', reason: client bundles registered at deploy; schema functions live in the catalog}
  surface/registration/unit/bundle: {title: A named client release holding its queries and schema version, reason: bundles are the unit of registration and of version pinning}
  surface/shared-fields/header: {title: 'Fields declared on the sum, present in every constructor', reason: follows model/sum-header}
  surface/source/op-log: {title: The ordered log of migration operators; the schema is its fold, reason: the schema is the fold of its operator log; operators are the primitive and carry inverses}
  surface/sum-decl/alternatives: {title: One declaration listing constructors separated by bars, reason: 'constructors listed as alternatives in one declaration, since sums are closed'}
  surface/sum-encoding/stable-tag: {title: 'A tag carrying the constructor''s stable id, then the payload', reason: the wire tag is the never-reused constructor id}
  surface/version-serving/translate: {title: 'Bundles keep running at their pinned version, translated through operator inverses', reason: 'operators are closed under inverses so that old versions can be served; every operator, not only add, is then online'}
  surface/version-serving/untranslatable/translate-at-apply: {title: Every registered query is translated when the operator applies; an untranslatable one refuses it, reason: 'refusal moves to the one place translation fails, and names the query'}
  surface/version/counter: {title: A per-database counter incremented by each migration, reason: one database has one linear operator log; its position is the version}
  surface/version/history/linear: {title: History is a chain; the later branch rebases its operators, reason: follows from the linear log}
  surface/wire/binary: {title: A binary protocol with per-query type descriptors, reason: a binary protocol for the server and an in-process API for embedding}
  surface/wire/embedded: {title: An in-process library linking the engine, reason: a binary protocol for the server and an in-process API for embedding}
  syntax/aggregate/comprehension: {title: agg(expr for x in q), reason: canonical notation}
  syntax/constraint/named: {title: Every constraint and check is named, reason: canonical notation}
  syntax/ctor-set/paren: {title: r.C and r.(C | D), reason: canonical notation}
  syntax/decl/type-relation: {title: type and relation, reason: canonical notation}
  syntax/equality/double: {title: Double equals, reason: canonical notation}
  syntax/keys/on-relation: {title: Clauses on the relation declaration, reason: canonical notation}
  syntax/module/header: {title: module name at the head of a file, reason: canonical notation}
  syntax/operator/verb: {title: 'Verb, object, clauses', reason: canonical notation}
  syntax/params/names: {title: By declared name, reason: canonical notation}
  syntax/query-decl/arrow: {title: Arrow before the result type, reason: canonical notation}
  syntax/record/colon: {title: Colon between field and value, reason: canonical notation}
  syntax/reference/key: {title: Key r or Key r.C, reason: canonical notation}
  syntax/strings/double: {title: Double quotes only, reason: canonical notation}
  types/calendar-overflow/explicit: {title: The operation takes the overflow rule as an argument, reason: the operation takes its overflow rule}
  types/collation/sortkey-fn: {title: Text orders by code point; a collation is a function to an ordered sort-key value, reason: 'text orders by code point; a collation is a versioned function to a sort key, indexable as an expression'}
  types/collation/version/versioned-name: {title: A collation name includes its version; moving to a new one is a schema operator that rebuilds, reason: a collation name includes its version; moving is a schema operator that rebuilds}
  types/decimal/scale-typed: {title: Scale is part of the type; result scales follow static rules and division names its scale, reason: scale is in the type; division names its scale}
  types/durations/split: {title: An exact Duration and a calendar Period are distinct types, reason: exact Duration and calendar Period are distinct types}
  types/eq-types/all-first-order: {title: Every value type has a derived structural ==, reason: every value type has structural ==}
  types/eq/leibniz: {title: Equal values are indistinguishable by every operation, reason: deduplication and planner substitution are unobservable only if equal values are indistinguishable}
  types/fault-forms/checked: {title: Checked operators return Option, reason: checked operators return Option; try turns a fault into a Result}
  types/fault-forms/try: {title: A try expression turns a fault into a Result, reason: checked operators return Option; try turns a fault into a Result}
  types/fault/abort: {title: The statement aborts with an error and its transaction cannot commit it, reason: the statement aborts; only total expressions may move past guards}
  types/fault/reorder/ordered-guards: {title: 'Guards order evaluation, and only total expressions move past them', reason: guards order evaluation; only total expressions move past them}
  types/float-eq/nan-greatest: {title: All NaNs equal and above every number; -0.0 equals 0.0, reason: == must be the equivalence the index encoding already implements}
  types/float-eq/zero/canon-store: {title: 'Values are canonicalized on construction, so -0.0 never exists in the value domain', reason: '-0.0 is canonicalized on construction, so it never exists as a value'}
  types/generics/bounded: {title: Type parameters may require capabilities such as equality or order, reason: 'parameters may require Eq, Ord, Hash or Num'}
  types/hash/key-bytes: {title: The hash of a value is a hash of its canonical key encoding, reason: 'equal values have equal canonical bytes, so hashing them is consistent with =='}
  types/id-type/key-type: {title: The field has the same Key type its referrers hold, reason: the key field has the Key type its referrers hold}
  types/infer/bidir-local: {title: Bidirectional checking with local type-argument synthesis, reason: signatures are written and generators fix binder types; errors stay local}
  types/int-overflow/trap: {title: Overflow is a fault, reason: overflow is a fault}
  types/int-widths/i64: {title: I64, reason: I8 to I64; unsigned and I128 left to Decimal and newtypes}
  types/int-widths/sized: {title: 'I8, I16, I32 beside I64', reason: I8 to I64; unsigned and I128 left to Decimal and newtypes}
  types/join/lub-exact-fields: {title: Least upper bound of constructor sets and keys; record arms must have the same fields, reason: constructor sets and keys meet at their union; anonymous records are exact}
  types/keygen/import/advance: {title: Writing an explicit value advances the generator past it, reason: an explicit value advances the generator past it}
  types/keygen/leak/live: {title: Live with exposed creation order, reason: the schema chooses its generator; uuidv7 leaks only creation time}
  types/keygen/promise/distinct: {title: Distinct values only, reason: under SSI no order is cheap except distinct}
  types/keygen/sequence: {title: A per-relation integer counter, reason: right-edge appends in the B-link tree; uuidv7 where ids must be minted without the database}
  types/keygen/uuidv7: {title: Time-ordered UUIDv7, reason: right-edge appends in the B-link tree; uuidv7 where ids must be minted without the database}
  types/literals/polymorphic: {title: 'A literal takes the numeric type its context expects, with a default', reason: a literal takes the numeric type its context expects}
  types/now/param-only: {title: No clock in queries; times come from parameters or generated fields, reason: queries are pure; time comes from parameters or generated fields}
  types/num-mix/explicit: {title: Operands must have one type; conversions are written, reason: conversions are written}
  types/sub-records/exact: {title: Record types are equal or unrelated, reason: anonymous records are equal or unrelated}
  types/text-eq/nfc: {title: Text is normalized to NFC on construction and compared exactly, reason: normalise on construction so equal text is equal bytes}
  types/text-eq/unassigned/pin-unicode: {title: The database records its Unicode version; upgrading renormalizes as a migration, reason: the database records its Unicode version; upgrading renormalizes as an operator}
  types/text-length/explicit: {title: 'No length; byte, scalar and grapheme counts are separate functions', reason: 'byte, scalar and grapheme counts are separate functions'}
  types/time-precision/micros: {title: Microseconds in an I64, reason: 'microseconds in an I64, as Arrow timestamp[us] and Postgres'}
  types/time-scale/smeared: {title: Every day has 86400 seconds; leap seconds are smeared or ignored, reason: every day has 86400 seconds}
  types/tzdb/catalog: {title: The rules are versioned catalog data; adopting a new release is an operation, reason: zone rules are versioned catalog data; adopting a release is an operator}
  types/variance/derived: {title: Variance inferred from where a parameter occurs in the declaration, reason: variance follows from where a parameter occurs}
  types/zoned/zoned: {title: A zoned date-time of civil time plus a named IANA zone, reason: civil time plus a named zone; offsets are a record}
  union-index/drop-ctor/after-vacuum: {title: 'Keep the removed heap as history until vacuum has cleared its entries, then drop it', reason: older snapshots and time travel still see the removed constructor's rows; vacuum clears entries in its batched pass}
  union-index/vacuum/batched: {title: 'Collect dead TIDs from every constructor heap, one index pass', reason: 'vacuum is per sum, one index pass'}
  write/bulk/from-query: {title: 'Insert, update and delete take a query as their source', reason: change sets are computed by queries}
  write/delete-action/cascade: {title: Delete the referrers, reason: 'refuse, propagate, or move the referrer to a constructor without the reference; no NULL'}
  write/delete-action/restrict: {title: Refuse the delete while referrers exist, reason: 'refuse, propagate, or move the referrer to a constructor without the reference; no NULL'}
  write/delete-action/retag-referrer: {title: Retag the referrer to a declared constructor without the reference, reason: 'refuse, propagate, or move the referrer to a constructor without the reference; no NULL'}
  write/insert/ctor: {title: The target is a constructor relation, reason: 'insert into a constructor relation, or into the sum with constructor values computed by a match'}
  write/insert/returns/rows: {title: 'The inserted rows, as a relation of the target''s type', reason: 'closure: a write returns the relation it inserted'}
  write/insert/value: {title: The target is the sum and each row is a constructor value, reason: 'insert into a constructor relation, or into the sum with constructor values computed by a match'}
  write/nested/rebuild: {title: 'A match rebuilds the value, copying the arm with one field changed', reason: a value is replaced whole or rebuilt by a match; no partial paths into sums}
  write/nested/replace: {title: The field is assigned a whole new value, reason: a value is replaced whole or rebuilt by a match; no partial paths into sums}
  write/primitive/delta: {title: A pair of inserted and deleted relations per constructor, reason: 'a write denotes inserted and deleted relations per constructor, the form constraints are checked on'}
  write/retag/assign: {title: Assigning a constructor value to the row; retag is a different constructor, reason: retag is assigning a value of another constructor; no separate construct}
  write/retag/old/in-scope: {title: Old fields are readable while computing the new payload, reason: the new value is computed from the old row}
  write/surface/txdata: {title: Queries compute change sets that a transaction applies, reason: queries stay pure; a write is a change-set value a transaction applies}
  write/transact/block: {title: A transact block computes a change set and a result from one snapshot, reason: a transact block is the form; registered transactions are named blocks; interactive transactions span several calls under SSI}
  write/transact/interactive: {title: 'A client transaction spans several registered calls, committed by the client', reason: a transact block is the form; registered transactions are named blocks; interactive transactions span several calls under SSI}
  write/transact/registered-txn: {title: 'Registered transaction functions, called as one round trip', reason: a transact block is the form; registered transactions are named blocks; interactive transactions span several calls under SSI}
  write/unit/multi: {title: A multiple-assignment block applied as one write, reason: 'one change set over several relations is one multiple assignment, checked once'}
  write/update/ctor-binder: {title: The update binds a constructor relation and sets its fields, reason: 'update a constructor relation, or the sum per arm of a match'}
  write/update/match-set: {title: The update binds the sum and sets fields per arm of a match, reason: 'update a constructor relation, or the sum per arm of a match'}
  write/upsert/cross-ctor/match-else: {title: The conflicting row is bound as the sum and the else branch matches on it, reason: the conflicting row is the sum and is matched like any sum value}
  write/upsert/unless-conflict: {title: An insert with an else branch run when a named key collides, reason: an insert with an else branch on a named key}
defeat:
  btree/ctor-rewrite/heap-alias: {title: 'A constructor may own several heaps, old heaps and entries stay', reason: one heap per constructor is meant literally}
  encoding/jsonb: {title: Shared columns plus a jsonb payload per constructor, reason: loses integrity on inner fields}
  encoding/tag-variants: {title: 'Parent table with stored tag, one table per constructor', reason: totality is not declarative with a stored tag}
  encoding/variants-only: {title: 'Constructor tables only, the sum a union view with derived tag', reason: exclusivity and FKs to the sum need emulation}
  encoding/wide: {title: 'One wide table, CHECKs tie nullable columns to the tag', reason: fields are nullable columns guarded by CHECKs}
  evolution/log-ops: {title: 'enable, deprecate and retire are operators with versions like any other', reason: retire in the log diverges embedded files and fleet tenants from their shipped migrations}
  evolution/remove-rows/reserve: {title: Reserve the removed name against reuse, reason: 'constructor ids are never reused; names carry no identity, and recompile catches stale references'}
  evolution/split-routing/catch-all: {title: Unrouted rows go to an added catch-all constructor, reason: expressible as a final arm of the routing match}
  evolution/split-routing/reject: {title: Fail the migration on any unrouted row, reason: a routing match checked exhaustive leaves no unrouted row}
  fk/retag/delete-insert: {title: 'Constructor change is delete plus insert, firing ON DELETE', reason: fires ON DELETE on FKs to the sum whose target row still exists}
  index/sum-or-ctor: {title: Only the whole sum or a single constructor, reason: 'two special cases of ctor-set, and subsets still need a third mechanism'}
  lang/offset: {title: Pages skip a number of rows, reason: 'sets have no positions, and offsets shift under concurrent writes'}
  layout/tag-value/stable-ordinal: {title: A per-sum ordinal assigned once at declaration and never reused, reason: a second per-sum id space duplicates the catalog id with no gain once ids are log-deterministic}
  project/spec/normative/file-format: {title: The on-disk file format, reason: the spec states guarantees over constructor relations; pages are implementation}
  query/narrowing: {title: A narrowing primitive that filters to one constructor, reason: same as ctor-relations at relation level; match covers value level}
  query/outer-join: {title: Outer joins of every constructor and CASE on the tag, reason: branches on nullness and multiplies rows; not the semantics}
  query/plain-sql: {title: Plain SQL over the union view and CASE on the tag, reason: we're not using any form of SQL; we're building our own database}
  sum/disjoint-partial: {title: 'Disjoint, totality optional per sum', reason: native sums have no parent row to be partial; partiality is a 0..1 relationship to a sum}
  sum/eer: {title: 'Full EER specialization, overlap and partiality allowed', reason: overlap is facets through relationships; no second specialization concept}
  sum/open: {title: 'Open, constructors may be added elsewhere', reason: extensibility belongs to evolution; an open sum gives up exhaustiveness everywhere}
  sum/owner-closed: {title: 'Closed to the schema owner, open to consumers', reason: conflates version skew with closure; stale clients are an evolution problem}
  surface/codegen/builder: {title: A typed query builder in the host language, reason: 'a builder-composed query is invisible until it runs, contrary to registered queries'}
  surface/read-only: {title: Ad hoc is read-only; fixes are migrations or registered transactions, reason: a registered bundle or migration per data fix makes loads and staging repairs impractical}
  surface/registration/on-change/retire: {title: The migration applies and failing queries are refused at call time, reason: 'a breaking change is refused; retiring a bundle is a separate explicit command, never an outcome of a migration'}
  types/float-eq/ieee: {title: 'IEEE comparison, with NaN unequal to itself', reason: '== is not reflexive, so sets and keys have no equivalence'}
  union-index/drop-ctor/eager: {title: Scan the dropped heap and delete its entries one by one, reason: deleting entries at remove loses versions that older snapshots and the retention window still see}
  union-index/retag/forbid: {title: 'No in-place retag, constructor changes are delete plus insert', reason: a semantic restriction imposed by physical layout}
  union-index/retag/live: {title: Live with cross-partition update semantics, reason: retag is an ordinary update; the engine moves the row atomically like any update}
  write/bulk/for-each: {title: A loop over a query whose body is a single-row write, reason: a row-at-a-time loop contradicts the change-set denotation}
  write/delete-action/deferred-restrict: {title: Refuse at commit if referrers still exist, reason: multiple assignment already checks at the statement boundary}
  query/tooling.other: {reason: no SQL surface}
  sum/level.other: {reason: value and relation are the only places a sum can sit; EER categories reduce to relation-level sums of references}
  union-index/retag.other: {reason: 'move, forbid and live cover the ways a cross-heap update can behave'}
defer: [btree/compress, btree/keyenc/lossy, btree/split, buffer/async/frames, buffer/async/order, buffer/replace, buffer/scan, exec/backend, layout/compress, layout/page-size]
---

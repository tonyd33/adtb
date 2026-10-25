# Execution

The planner turns normalised kernel terms ([kernel.md](kernel.md)) into
physical plans; the executor runs them over the heaps of
[mvcc.md](mvcc.md) and the indexes of [indexes.md](indexes.md). The
catalog stores the plans of registered queries.

## 1. Physical plans

A physical plan is a set of pipelines through which producers push
batches into consumers. A lowered `match` is a union of arm queries;
each arm is its own pipeline, `scan(heap) -> arm -> sink`, and the
union is the shared sink.

Invariant: a plan evaluates each call of a nondeterministic host
function in the query's meaning exactly once; the planner never merges
or repeats such calls
([../spec/interop.md §9.3](../spec/interop.md#93-host-functions)).

Rulings: exec/flow/push

## 2. Batches

Batches between operators are Arrow record batches, held in the
engine's own buffers (`adtb::batch`). This is the engine's internal
format; the wire's Arrow format
([../spec/wire.md §5.4](../spec/wire.md#54-arrow-format)) is a separate
contract.

A batch from a constructor heap has that constructor's schema, one
column per field in declaration order, and no validity bitmaps other
than those of the `Option` layout below, since no field is NULL.

| Type | Column |
| --- | --- |
| `I8`, `I16`, `I32`, `I64`, `F32`, `F64` | Fixed width of the same size |
| `Decimal s` | Fixed width, the coefficient as a 16-byte `i128` |
| `Instant`, `Time`, `DateTime`, `Duration` | Fixed width `i64` microseconds |
| `Date` | Fixed width `i32` days |
| `Text`, `Bytes`, `SortKey` | Variable-length binary, `i32` offsets |
| `ZonedDateTime` | Struct of the `i64` local time and the zone as `Text` |
| `Period` | Struct of the `i32` months and the `i32` days |
| A record | Struct of its fields in declaration order |
| A newtype | Its representation's column |
| `Key r.S` and a reference | The column of `r`'s key type |
| `Set T` in a nested result | List of `T`, `i32` offsets |
| A sum | By its shape, below |

A value-level sum field is an Arrow dense union: an `i8` type-id
buffer, an `i32` offsets buffer, and one compact child per constructor,
the child a struct of the constructor's fields. Three shapes are
special-cased, chosen in this order:

| Sum shape | Batch layout |
| --- | --- |
| One constructor | Its payload, a struct of its fields, with no tag |
| No constructor has fields | The tag vector alone: each row's constructor position, as `u8` up to 256 constructors, `u16` up to 65 536, `u32` past that |
| `Option T`: two constructors, one nullary, and the other's present column takes a validity bitmap | The present column with a validity bitmap; a cleared bit is the nullary constructor |

The present column of `Option T` is the other constructor's one
field's column when it has one field, else the struct of its fields.
It takes a validity bitmap unless it is a dense union or itself has
one; `Option (Option T)` and `Option` of a sum laid out as a union are
dense unions.

These are engine layouts only; the language still sees constructors.
Type ids in a batch are positions in the batch schema, which maps each
to the constructor's stable id. Nested results are Arrow lists of
structs.

A union holds at most 127 constructors. A sum with more is a union of
unions: the outer union's children are unions of up to 127
constructors each, nested as deep as needed. With `n` constructors and
`g` the least power of 127 with `127 g >= n`, child `j` of the outer
union covers positions `j g` up to `(j + 1) g`, laid out by the same
rule, and a type id at each level is the child's index. The batch
schema's type map still names each leaf constructor by its stable id
([../spec/wire.md](../spec/wire.md)).

Every buffer of a batch satisfies Arrow's layout, so a consumer may
hand it to an Arrow implementation or write it as an IPC body without
copying:

- Its first byte's address is a multiple of 64.
- Its length padded to the next multiple of 64 is allocated, and the
  padding bytes are zero.
- Values are little-endian. Offsets of a binary or list column are
  `len + 1` values from 0, never decreasing. A dense union's offsets
  are `len` values numbering each child's rows in turn from 0.
- A validity bitmap holds one bit per row, least significant bit
  first; a row whose bit is cleared still has a slot in the present
  column, holding an unspecified value.
- Every child of a struct has the struct's length.

Rulings: exec/batch/arrow, exec/value-sum/dense, exec/value-sum/special/option-validity, exec/value-sum/special/enum-tag, exec/value-sum/special/one-ctor

## 3. Executing match

Where the constructor is known per pipeline, an arm runs over its own
heap's batches. Where it varies per row (a value-level sum field, a
probe of an index over several constructors, or a merge of several
heaps), the batch is split by a single pass over the tag into one
selection vector per constructor, each arm runs over its selection,
and results are written back to their positions.

Arms whose sources are the same index range are fused: one probe of
the index, and each entry `(key, ctor, tid)` is routed by its
constructor to a fetch from that heap and that arm. This is a physical
operator; the meaning of the match is still the union of its arms
([../spec/semantics.md §5.5](../spec/semantics.md#55-sums-and-match)).

```adt
from i in identities where i.id == k
select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub
```

```text
IndexScan key index (id = k)
  -> Route by ctor { EmailPass: Fetch(EmailPass heap) -> arm 0
                     Google:    Fetch(Google heap)    -> arm 1
                     Apple:     Fetch(Apple heap)     -> arm 2 }
```

Rulings: query/native, exec/interleaved/sel-vectors, exec/probe/fuse

## 4. Pruning

Arms are pruned at compile time when the source's type excludes their
constructor or their body is empty, and at executor start when a
parameter fixes the constructor. A source typed `Key identities.OAuth`
never plans an `EmailPass` arm.

Postcondition: a stored plan names the heaps and indexes it reads
after compile-time pruning. The watermark
([mvcc.md §9](mvcc.md#9-the-watermark)) relies on it.

Plans are specialised per pattern of present `Option` parameters
([../spec/queries.md](../spec/queries.md)).

Rulings: exec/prune/compile, exec/prune/init

## 5. Stored plans

A registered query is planned and checked when it is registered, and
its plan is stored in the catalog with the schema version and the
statistics it was costed with; calls never plan. A plan names
constructor ids, so it stays valid while the constructor sets it reads
are unchanged.

A stored plan is replanned:

- when a sum it reads changes version;
- when statistics of a heap it reads drift past a threshold (for
  example a constructor grown from empty).

Plan pinning, regression reverts and replanning on a new engine
release are in [../spec/catalog.md §7](../spec/catalog.md#7-plans).

The planner uses an expression index for a query only when the code
identity of the query's expression equals the index's
([../spec/interop.md §9.5](../spec/interop.md#95-function-identity)).

TODO: the identity by which a plan and an expression index name the code of a user-defined function is undecided here; [../spec/evolution.md §7](../spec/evolution.md) and [../spec/interop.md §9](../spec/interop.md) state its observable effect.

Rulings: exec/plan-cache/at-register, exec/replan/schema, exec/replan/stats

## 6. Quarantine

A call whose read set
([../spec/queries.md §12.4](../spec/queries.md#124-the-constructors-a-statement-reads))
includes a quarantined heap fails, even where pruning (§4) removes that
heap's arms. A plan never uses a quarantined index: a stored plan that
uses one is replanned without it while the index is quarantined
([storage.md §8](storage.md#8-quarantine)). Queries whose read set
excludes the quarantined heap continue.

Rulings: ops/halt-scope/heap, spec/reads/static/static-types, spec/quarantine/index/cost-only

## 7. Notes for the implementation

Non-binding.

### 7.1 Adaptive compilation and morsels

Pipelines start in a bytecode interpreter. Per pipeline the engine
extrapolates the remaining work from morsel progress and compiles hot
pipelines of registered queries to machine code when the estimate says
compilation pays.

TODO: the code generator backend is deferred.

Parallelism is morsel-driven: worker threads take (pipeline, morsel)
tasks from a dispatcher, steal work, and write to thread-local output
areas. A morsel never spans heaps; several small constructor heaps' arm
pipelines are packed into one task that runs them in turn. Calls from
the server become tasks on the same scheduler
([server.md §1](server.md#1-calls-as-tasks)).

The engine checks for cancellation at morsel boundaries, so a call
stops within one morsel per worker. A result stream with no credit
stops its executing pipeline when the pipeline's output buffer is full.

Rulings: exec/model/adaptive, exec/parallel/morsel, exec/parallel/small-heaps/pack

### 7.2 Joins

Joining a matched sum with another input either distributes the join
into every arm, costing each on its own heap, or hoists it above the
union when the join key is a field every arm outputs. The optimizer
chooses per query. When the join is distributed, the other side is
built once into a hash table that every arm's pipeline probes.

```adt
from i in identities, u in users where i.user == key u
select { name: u.name, login: match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub }
```

```text
build: Scan(users) -> HashBuild(id)
arm 0: Scan(EmailPass) -> Probe -> sink
arm 1: Scan(Google)    -> Probe -> sink     -- or IndexNL on users' key index
arm 2: Scan(Apple)     -> Probe -> sink
```

Rulings: exec/join/cost, exec/join/rebuild/shared-build

### 7.3 Statistics

Statistics are kept per constructor heap: the row count, and per field
an online reservoir sample and a mergeable distinct-count sketch.
Statistics of a field over the whole sum are the merge of its heaps'
sketches. A match's cardinality estimate is the sum of its arms'
estimates, each from its own heap, since arms are disjoint.

Rulings: exec/stats/per-ctor, exec/stats-form/sketch, exec/arm-card/per-arm

### 7.4 Nested results and fixpoints

A nested result may be evaluated as a fixed number of flat queries;
the value is the nested one.

A `fix` may be computed by semi-naive iteration, or by any other
method that yields the sets of
[../spec/semantics.md §5.10](../spec/semantics.md#510-fixpoints).

### 7.5 Memory

Memory is split into a fixed page pool, sized at open, and per-query
memory. Each query obtains arenas on demand within a per-query budget
and frees them whole when it ends; an arena request past the budget
fails with an operating error of that query. The planner sizes hash
builds, sorts and groupings against the grant the query was admitted
with ([../spec/catalog.md §8.1](../spec/catalog.md#81-admission)), and
operators that exceed it spill to temporary pages of the pool, then,
when the pool has none to spare, to spill files in the spill directory.
Spill files of an encrypted database are encrypted under a key
generated per process and never stored
([../spec/security.md §9.1](../spec/security.md#91-what-is-encrypted)).

```text
let arena = budget.arena(query_id, limit)?;
let table: HashTable<'_, K, V> = HashTable::new_in(&arena);
```

Rulings: buffer/memory/split, project/arch/alloc/budgeted

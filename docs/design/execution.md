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

The planner is the module `adtb::plan`. This section states its input,
the plan it returns and how a plan is evaluated, which is all an
executor relies on.

Rulings: exec/flow/push

### 1.1 The planner's input

```text
fn plan::plan(schema: &Schema, catalog: &Catalog, terms: &Terms, root: Tm,
              params: &[Var], options: Options) -> Result<Plan, plan::Error>
```

Preconditions:

- `root` is a normal form in `terms` that `adtb_lang::norm` returned
  for `schema` ([kernel.md §7](kernel.md#7-normal-forms)), and its free
  variables are among `params`.
- `catalog` describes every relation variable `root` reads, as a
  `RelationDesc` per `RelId`:
  - `heaps` maps every constructor of the relation's sum to its heap id;
  - `key_index` is the id of the key index;
  - `indexes` lists the indexes the planner may use, the key index
    included. Each `IndexDesc` has its id, its constructor set, sorted,
    its fields in key order, each a field every constructor of the set
    has, whether it is `unique`, and whether it is `ordered`: a B-link
    tree, which a probe may fix a proper prefix of. A hash index is not
    ordered.
  - Heap ids are distinct, and so are index ids.
- An index whose key holds an expression or a path through a
  value-level sum is not in `catalog`; the planner uses none (§5).

`options.rewrites` turns each rewrite of §1.3, §3, §4, §7.2 and §7.4
on or off. `options.hiding` is set for a bundle declaring `hide new
constructors`, for which every dereference is partial.

Postconditions:

- On `Ok(p)`, every outcome of evaluating `p.root` (§1.3) with
  `p.params` bound to the parameters' values is an outcome
  [../spec/semantics.md](../spec/semantics.md) allows `root`, whatever
  `options` holds.
- `Err(UserCode(name))` when `root` calls the Wasm or host function, or
  uses the user-defined aggregate, `name`: plans do not hold user code
  yet (§5).
- `Err(NotAQuery(_))` when `root` is or holds a write, a block, a read
  of a statement's transition, or `at`.
- `Err(Internal(what))` when `root` breaks an invariant the
  preconditions guarantee, such as a function value or a dereference of
  a term that is not a key; `what` names the invariant. The caller fails
  the statement with `Internal { what }` as it fails one on a fault,
  and the transaction stays open.

```text
fn plan::plan_bounded(schema: &Schema, catalog: &Catalog, terms: &Terms, root: Tm,
                      params: &[Var]) -> Result<Plan, plan::Error>
```

Preconditions: those of `plan`.

Postconditions: those of `plan` with every rewrite on, and:

- On `Ok(p)`, `p` has no `Scan` source and no `Build`: it reads every
  constructor relation through an `Index` end or a `Lookup` whose key
  holds at least one value, bound before the read. A read of `ctors`
  that no one index covers is a `Fork` with one read per constructor
  of `ctors`, each through an index covering that constructor.
- `Err(Unbounded { rel, ctors, fields })` when some read of `ctors` of
  `rel` has no such probe, neither over `ctors` nor over each of its
  constructors; `fields` are the fields of the read row that
  the equalities among its conditions fix, in order of first mention,
  empty when none does.
- `Err(Outside("fix"))` when `root` holds a `fix`.

### 1.2 Plan structure

| Type | Fields | Meaning |
| --- | --- | --- |
| `Plan` | `terms`, `params`, `root: Eval`, `ty`, `reads: Reads` | The value of `root` with `params` bound, of type `ty`: the type of the term the planner was given |
| `Reads` | `heaps`, `indexes` | Every heap and index the plan names (§4) |
| `Eval` | below | The computation of one value in an environment |
| `Query` | `builds`, `spine`, `sink`, `total` | Pipelines that push values into one sink |
| `Spine` | below | The choices taken once per evaluation of a query that select its pipelines |
| `Pipeline` | `source`, `chain` | Rows from the source, each passed through the chain |
| `Chain` | `steps`, `end` | Steps applied to each row in turn, then the end that consumes it |
| `Build` | `var`, `key`, `pipelines` | A hash table over the distinct values its pipelines emit |
| `Fix`, `FixBind` | below | A simultaneous least fixpoint |

An expression is a `Tm` of `Plan.terms`, an arena of
[kernel.md](kernel.md) terms. Every expression a plan names is scalar:
its forms are `Var`, `Const`, `Prim`, `Record`, `Field`, `Ctor`,
`Case`, `If`, `Let`, `Caller`, `Key`, `KeyOf`, `Tag`, `Try`,
`Singleton`, `EmptySet`, `Union`, `Restrict` and `Unreachable`, each
with the meaning and typing invariants of kernel.md §3. A scalar
expression reads nothing; every read of a plan is a `Scan`, an `Index`
end, a `Lookup` or a `Build`. `Plan.terms` holds the terms the planner
was given, so a variable of the input keeps its `Var`.

An environment binds variables to values. `Plan.root` is evaluated in
the environment binding each variable of `Plan.params` to its
parameter's value.

| `Eval` | Value |
| --- | --- |
| `Expr(t)` | The value of expression `t` |
| `Let(x, a, b)` | `b`'s, with `x` bound to `a`'s |
| `If(c, a, b)` | `a`'s when `c` is `True`, else `b`'s |
| `Case(s, arms)` | The arm's for the constructor of `s`'s value, its variable bound to that value; one arm per constructor `s` may hold |
| `Try(a)` | `Ok { value }` of `a`'s value, or `Err { error }` of the `Fault` it raises; any other abort passes through |
| `Lookup { index, key, routes }` | The row whose `index` entry has the field values `key`, fetched from the heap its constructor routes to; fails with `NoSuchRow` when there is none or its constructor has no route. `index` is the relation's key index, and `key` one value per key field |
| `Query(q)` | The value `q`'s sink forms (below) |
| `Fix(f)` | As below |
| `GroupRows { groups, key }` | The `rows` of the record of the set `groups`, a value a `Group` sink with `rows` set formed, whose `key` equals `key`'s value; the empty set when none does |
| `At { instant, point, pins, body, heaps }` | `body`'s, evaluated at the state `point`'s value names, a `Position`, or an `Instant` when `instant` is set, for a block typed at version `pins` or, when it is `None`, at the version of that state; `heaps` are the heaps `body` reads. Fails with `HistoryUnavailable { oldest }` or `WrongVersion { version }` as [mvcc.md §10.3](mvcc.md#103-history-heaps-and-reads-of-the-past) states |

A query is evaluated in an environment:

1. A `Top` sink's `count` is evaluated first.
2. The spine is evaluated, which selects pipelines, each with the
   environment it runs in.
3. Every selected pipeline runs. A build is made before a selected
   pipeline probes it, at most once per evaluation of the query, in the
   query's environment; a build no selected pipeline probes is not made.
4. The sink forms its value from the set of values the pipelines
   emitted: duplicates are one value.

| `Spine` | Pipelines selected |
| --- | --- |
| `Pipeline(p)` | `p`, in the spine's environment |
| `Union(ss)` | Those of every `s`; none for `Union([])` |
| `If(c, a, b)` | `a`'s when `c` is `True`, else `b`'s |
| `Case(s, arms)` | The arm's for the constructor of `s`'s value, its variable bound to that value |
| `Let(x, e, s)` | `s`'s, with `x` bound to `e`'s value |

| `Source` | Rows |
| --- | --- |
| `Unit` | One row, binding nothing beyond the environment |
| `Scan { heap, ctor, var }` | One per row of `heap`, whose constructor is `ctor`, binding `var` to it |

| `Step` | Effect on each row |
| --- | --- |
| `Filter(c)` | Kept when `c` is `True`, dropped when `False` |
| `Bind(x, e)` | `x` bound to `e`'s value |
| `Unnest(x, s)` | Replaced by one row per element of the set `s`, binding `x` to it |
| `Probe { build, key, var }` | Replaced by one row per element of the query's build `build` whose key values equal the values of `key`, binding `var` to it; an empty `key` matches every element |

| `End` | Effect on each row |
| --- | --- |
| `Nothing` | Dropped |
| `Emit(t)` | `t`'s value pushed into the query's sink |
| `Fork(cs)` | Passed to every chain of `cs` |
| `If(c, a, b)` | Passed to `a` when `c` is `True`, else to `b` |
| `Case(s, arms)` | Passed to the arm for the constructor of `s`'s value, its variable bound to that value: the per-row routing of §3 |
| `Index { index, key, var, routes }` | Probes `index` for the entries whose leading fields equal `key`'s values, one value per field of a prefix; each entry whose constructor has a route is fetched from the route's heap and the row, with `var` bound to it, passes to the route's chain; other entries are dropped |
| `Range { index, key, lo, hi, var, routes }` | As `Index`, for the entries whose field after the prefix `key` fixes also lies above `lo`'s value and below `hi`'s in the language's order, each bound inclusive when its `inclusive` is set and absent when it does not bound |

A `Build` is made by running its pipelines, which emit its elements;
each element's key is the values of `key` with `var` bound to it.

| `Sink` | Value formed from the set `S` of emitted values |
| --- | --- |
| `Set` | `S` |
| `Exists`, `IsEmpty` | Whether `S` is nonempty, empty |
| `Agg { op, var, body, ty, body_ty }` | The aggregate `op` of `body`'s values, one per element of `S` bound to `var`, at `ty`, by [../spec/semantics.md §5.7](../spec/semantics.md#57-aggregates); `body_ty` is `body`'s type, and `ty` is `Option body_ty` exactly when a `min`, `max` or `avg` may range over nothing |
| `Group { var, by, rows, aggs }` | One record per distinct value of `by` over the elements of `S` bound to `var`: `key`, the value; `rows`, when `rows` is set, the elements with that key; and one field per `GroupAgg`, its `field` |
| `Top { count, keys, var }` | The first `count` elements of `S` ordered by `keys`, each `(key, descending)` over `var`; fails with `NegativeCount`, after `S` and the keys are computed, when `count` is negative |
| `Present { var, keys, take, after }` | The elements of `S` ordered by `keys` as a sequence, past the cursor `after` and at most `take` of them, as `Present` of kernel.md §3; `take` and then `after` are evaluated after the keys |

A record of a `Group` lists its fields sorted by name. A `GroupAgg`
computes `op` per group at `ty`, over the set of values of `element`,
with `elem` bound to each, for each row of the group bound to `row`
that passes every `filter` in turn; its `body`, with `elem` bound, is
the aggregated value, of type `body_ty`.

`Fix { binds, body }` binds each `FixBind.var` to a set and evaluates
`body`. With any `step` absent, the sets start empty and each round
evaluates every `body` of `binds` on the previous sets, until a round
changes none, as
[../spec/semantics.md §5.10](../spec/semantics.md#510-fixpoints)
states. With every `step` present the evaluation is semi-naive: a
first round evaluates every `body` on empty sets and makes each result
both the set and its `delta`; each later round evaluates every `step`
with the sets and the deltas bound, takes as the new delta the
elements not already in the set and adds them, and the rounds stop
when every delta is empty. A fixpoint that has not stopped within the
statement's limits is rejected for its limit.

### 1.3 Invariants and evaluation

Invariants of every plan:

- Every variable an expression mentions is bound where the expression
  is evaluated: by `Plan.params`, a `Let`, `Case` or `Spine` binder, a
  source, a step, a route, a sink's or a `GroupAgg`'s variable, or a
  fixpoint's variable or delta.
- `Filter`, `If` and every `End::If` condition is `Bool`; every `Case`
  names each constructor its scrutinee may hold once.
- A `Probe`'s `build` indexes the `builds` of the query whose spine
  holds the pipeline. A build's pipelines have `Scan` sources, `Filter`
  steps and an `Emit` or `Nothing` end, and its pipelines and `key`
  mention only its scan variable and variables of its query's
  environment.
- A `Scan`'s `heap` is `ctor`'s heap in the catalog. An `Index` or
  `Range` end's routes name distinct constructors of the index's
  constructor set, each with that constructor's heap; so do a
  `Lookup`'s.
- A `Range` end's index is `ordered`, its `key` is shorter than the
  index's fields, the field after them is a field of the row, not an
  expression, and `lo` and `hi`, when present, are of that field's
  type, which is ordered.
- `total` is set only when no evaluation of the query's pipelines and
  builds, nor of its sink's `body`, `by` or keys, aborts, whatever the
  database holds; aborts for the statement's limits and cancellation
  excepted. A `Spine::Let` the planner adds binds a value that
  evaluation computes without aborting.
- `Plan.reads` holds every heap a `Scan`, route or `Lookup` names and
  every index an `Index` or `Range` end or `Lookup` names, including
  those of builds and nested queries.
- An `At`'s body names no `Index` or `Range` end and no other `At`,
  and its `heaps` holds every heap its body names. A query holding an
  `At` holds no build or `Spine::Let` computed from its body.

Evaluation of a plan:

- The executor evaluates every row of every selected pipeline and every
  step of every chain a row reaches, with two exceptions for a query
  whose `total` is set: an `Exists` or `IsEmpty` sink may stop its
  pipelines once one value is emitted, and a `Top` sink whose `count`
  is zero or negative may form its value without running them. A query
  whose remaining rows could abort reports the abort. With rewrite
  `early_stop` off, no query is `total`.
- Rows of one pipeline, the pipelines of one query and the elements of
  a sink may be processed in any order, interleaved or in parallel. An
  abort of any evaluation is the outcome of the query, and when several
  abort, any one of them is
  ([../spec/semantics.md §1.3](../spec/semantics.md#13-conformance)).
- The steps of a chain are applied to a row in order, and an `Eval`'s
  parts in the order its row of the table above states. The planner
  keeps every partial term after the guards that dominated it in
  `root` ([kernel.md §7.1](kernel.md#71-totality)).
- Cardinalities and the order of an aggregate's fold are free: the
  executor's choices are bound only by
  [../spec/semantics.md §1.3](../spec/semantics.md#13-conformance).

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

In a plan (§1), a comprehension over a constructor relation has one
pipeline per heap, each scanning its heap with its constructor fixed. A
`case` over that pipeline's row, as a chain's end, a condition or a part
of an expression, is replaced by its arm when the plan is built
(rewrite `static_route`). Where the constructor varies per row, a chain
ends in `End::Case`, whose arms an executor runs over selection
vectors. A fused probe is one `End::Index` whose routes name every
constructor the comprehension reads (rewrite `fuse_probe`); unfused,
each constructor's pipeline probes the index on its own. Both are
physical choices with the same outcomes.

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

In a plan (§1), with rewrite `prune` on:

- a comprehension over a constructor relation has pipelines, routes and
  build pipelines only for the constructors of its source's constructor
  set;
- a pipeline or route whose chain is `Nothing` with no step is left
  out: one whose conditions reduce to `False` for its constructor, or
  whose body emits nothing, reads no heap.

A condition or a case scrutinee under a comprehension at the start of a
query that does not depend on the comprehension's row, and that is
total, is moved out of it into the query's spine (rewrite `unswitch`),
so that the spine's `Case` or `If`, evaluated once before any pipeline
runs, selects the pipelines of one arm. A `match` over an `Option`
parameter in a `where` clause becomes a spine `Case` whose arms hold
the pipelines of each pattern of the parameter: that is the
specialisation per pattern. A partial condition is never moved past
the conditions before it.

`Plan.reads` is the postcondition above: the heaps and indexes of the
plan's pipelines after compile-time pruning, every arm of a spine
included.

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

The interpreter is `adtb::exec`. `exec::compile` turns a plan into a
program: each scalar expression becomes register instructions, each
primitive an operation chosen for its operand types, and each field
access or key a position in the row. An instruction runs over every
selected row of a frame, which holds one column per variable; `if`,
`case` and `try` split the selection, run each arm over its rows and
write the results back to their positions. A plan that uses a primitive
or a form the interpreter lacks is refused when it is compiled, and an
operation it lacks for some values only, such as normalizing a
concatenation to NFC, fails the statement when it meets them.

A morsel is a batch of at most `morsel_rows` rows of one heap that the
call's snapshot sees, in the batch layout of §2; its rows become the
column of the scan's variable. An index probe or lookup walks each
entry's version chain to the version the snapshot sees, and drops it
when it lies in another heap or its key fields no longer equal the
probe's.

Parallelism is morsel-driven: worker threads take (pipeline, morsel)
tasks from a dispatcher, steal work, and write to thread-local output
areas. A morsel never spans heaps; several small constructor heaps' arm
pipelines are packed into one task that runs them in turn. Calls from
the server become tasks on the same scheduler
([server.md §1](server.md#1-calls-as-tasks)).

`exec::Scheduler` starts its workers through the runtime, so the
simulator schedules them. They take tasks from one shared queue, and a
call's own thread runs queued tasks while it waits for its query; no
worker keeps a queue of its own to steal from yet. The pipelines of a
query evaluated once per statement become tasks: the scan of a heap is
one task per range of `range_pages` of its pages, which runs a morsel at
a time over the rows of its range and reads a page only once the
morsels before it have run, and pipelines with no source or over
heaps of at most `pack_pages` pages run in turn in one task. Rows a
step or an end expands, by `Unnest`, `Probe`, `Index` or `Range`, go
on in frames of at most `morsel_rows` rows. Each task emits into its
own set, merged into the query's when the task ends. A query evaluated
per row of a pipeline runs on that
row's thread. The first task that probes a build makes it, while the
others wait for it.

The engine checks for cancellation at morsel boundaries, so a call
stops within one morsel per worker. A result stream with no credit
stops its executing pipeline when the pipeline's output buffer is full.
The executor checks a call's cancel flag before each morsel and each
round of a fixpoint, and once one task of a query fails, the query's
tasks not yet started do not run. Results are formed whole; no credit
reaches a pipeline yet.

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

The planner distributes every join. A comprehension over a constructor
relation inside a pipeline is read in one of three ways:

- an `Index` end on an index whose leading fields the conditions fix by
  equality to values computed from the row (rewrite `index_join`);
- a `Probe` of a build keyed by the conditions' equalities between an
  expression over the inner row and one over the outer row (rewrite
  `hash_join`);
- a `Probe` of a build with no key, a nested loop, which is always
  available.

The conditions are a comprehension's `where` conjuncts in order. Only
the longest prefix of them that is total and reads nothing may be
reordered: those that mention the inner row and no variable bound in
the query go into the build, those that do not mention the inner row
run before the probe, and the equalities become keys. The first partial
condition and every one after it run after the probe, in order. A
condition that does not mention the row of a comprehension nested
directly in another is moved to the outer one when it is total. The
same rules choose an index for a comprehension at the start of a query
(rewrite `index_scan`), whose key values may then mention anything but
the row.

With rewrite `range_probe` on, an ordered index whose leading fields
the equalities fix, none or some, is also read through a `Range` end
when a comparison `<`, `<=`, `>` or `>=` among the reorderable
conditions bounds the next field, a field of the row, by a value of the
field's type that does not mention the row. The first lower and the
first upper bound are taken; every comparison stays a filter on the
routes. A probe fixing more fields is preferred, then one with a range.
`plan_bounded` makes no `Range` end.

The build of a join is keyed by the relation, its constructor set, the
row variable, the key and the filters, so arms that share the joined
subterm probe one build.

Statistics (§7.3) do not exist yet. The planner uses a fixed cost
model:

| Quantity | Assumed |
| --- | --- |
| Rows of every heap | 1000 |
| Rows of a probe of an index that is not unique, or whose key does not fix every field, and of a range probe | 100 |
| Rows of a unique probe fixing every field | 1 |
| Rows of a set an `Unnest` ranges over | 10 |
| Fraction of rows a condition keeps | 1/2 |
| Cost of reading a row, and of one hash probe | 1 |
| Cost of one index probe | 4 |

A pipeline starts with one row per evaluation of its query. With `n`
rows reaching a comprehension over `k` constructors, an index join
costs `4 n`, or `104 n` when the probe is not unique; a hash join
`1000 k + n`; and a nested loop `1000 k + n m`, `m` the build's rows
after its filters. The cheapest is chosen. Hoisting a join above a
union costs the same probes as distributing it with one shared build,
so the fixed model never hoists.

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

The planner evaluates a nested result as a query per row of its
enclosing pipeline (`Step::Bind` of an `Eval::Query`), whose
comprehensions over constructor relations probe an index where their
conditions allow.

With rewrite `flatten` on, a query planned inside a chain whose source
is a comprehension over a constructor relation is flattened when its
reorderable conditions hold an equality between a term over the row and
a term over the enclosing row. The comprehension's rows that pass its
reorderable conditions mentioning only the row and variables bound
outside the enclosing query are grouped, once per evaluation of that
query, by the row's sides of those equalities: a `Group` query bound
by a `Spine::Let` of the enclosing query. Per enclosing row,
`GroupRows` looks up the group of the other sides' values, and the
nested query ranges over its rows with the remaining conditions in
their order. Its sink is unchanged. The fixed cost model chooses: a
flattened read costs `1000 k + n` against `n` per-row reads of `4`,
`104` or `1000 k`, so a nested result over one row is never
flattened. `plan_bounded` flattens nothing.

With rewrite `early_stop` on, a query's `total` is the totality of its
source and of its sink's per-element terms by
[kernel.md §7.1](kernel.md#71-totality). The executor stops an
`Exists` or `IsEmpty` query with `total` set once any task inserts a
value into its set: tasks not yet started do not run, running scans
stop at their next morsel, and rows waiting for a partitioned build are
not resumed. A `Top` query with `total` set and a count of zero or less
forms its value without selecting pipelines.

The planner gives a `fix` semi-naive steps (rewrite `semi_naive`) when
every occurrence of its variables in its bodies is a generator source
or a branch: under `union`, the source or body of a `for`, a `▷`, a
branch of an `if` or `case` whose condition or scrutinee does not
mention them, or the body of a `let` or inner `fix` whose bound terms do
not. The step of a body `e` is `δ(e)`, where `δ(X) = ΔX`,
`δ(e₁ ∪ e₂) = δ(e₁) ∪ δ(e₂)`,
`δ(for x in s do b) = (for x in δ(s) do b) ∪ (for x in s do δ(b))`,
`δ` passes through the other forms, and `δ(e) = {}` for an `e` that
does not mention the variables. Every set read in a step holds the
elements found so far, so a step yields every element the last round's
deltas can derive and nothing outside the fixpoint. A `fix` with any
other occurrence, such as one under `exists` in a positive condition, is
iterated naively.

The planner fuses a `count`, `min`, `max`, `avg` or `sum` over the
rows of a `group` into the group's sink (`GroupAgg`, rewrite
`group_agg`) when its range is a comprehension over the group's rows
that selects one element per row holding the row, and its terms
mention only variables bound outside the body of the comprehension
over the groups. An aggregate that may abort is fused only when it is
evaluated whenever the body is, it is the only one, and the body is
otherwise total, so the fused plan aborts exactly when the body does.
The group then keeps its rows only when the body still reads them.

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
let grant = budget.admit(bytes)?;   // None when the running grants leave too little
let mut arena = grant.take(n)?;     // StatementLimit past the grant
arena.grow(m)?;
```

`exec::Budget` is the query memory of the process. `Budget::admit`
grants a call its bytes, or admits nothing when the grants of running
calls and this one together would exceed the budget; the grant returns
to the budget when the call's last holder drops it. A call takes arenas
from its grant for what it holds: the rows of each morsel while the
morsel runs, the values its sinks collect, build tables, group records
and the sets of each fixpoint round. Each arena returns to the grant
when dropped. Plans carry no grant estimate yet.

Operators spill to one spill file per call, in the directory `spill`
beside the database, made on the first spill and removed when the call
ends; `Options::spill_bytes` bounds it. Spill buffers are outside the
grant, at most a chunk per writer or reader. Spilling goes straight to
the file: no temporary pages of the pool are used yet, and spill files
are not encrypted yet. A spill file that cannot be made, written or read
fails the statement with that I/O error, not `StatementLimit`.

- The values a query's pipelines emit are a distinct set that holds at
  most an eighth of the grant in memory and writes the rest as sorted
  runs. Sinks read the set back in order, a morsel at a time, merging
  at most 64 runs at once.
- A build holds at most a quarter of the grant in memory. Past that its
  elements are partitioned by key hash into 16 partitions of the spill
  file, or one when it has no key. A row that probes a partitioned
  build waits in its partition, with its key and the variables its
  frame binds row by row. When the query's pipelines have run, each
  partition is loaded in chunks of at most half the grant's room, and
  its waiting rows resume after the probe against each chunk in turn.
- A `Group` sink passes key and element pairs through a spilling
  distinct set and forms one group at a time; one group's elements must
  fit the grant.
- `Top` and `Present` keep only the elements they return.
- An `Agg` or `Fold` sink folds its values a morsel at a time.

A value a query returns, a group, a morsel and a fixpoint's sets do not
spill: an arena request for one past the grant, or a spill past
`spill_bytes`, rejects the statement with `StatementLimit`, which `try`
does not catch.

Rulings: buffer/memory/split, project/arch/alloc/budgeted

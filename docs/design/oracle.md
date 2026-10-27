# Oracle

`adtb-reference` is a naive evaluator of the kernel calculus of
[../spec/semantics.md](../spec/semantics.md) over in-memory sets. It is
the oracle of the test harness in `adtb-sim` and of the conformance
runner.

## 1. Evaluation contract

```text
Interpreter::new(schema, program, db)      // plus with_caller, with_change, with_fix_limit, with_closed,
                                           // with_files, with_row_limit
  .eval(terms, t, params) -> Result<Value, Abort>
  .apply(terms, t, args) -> Result<Value, Abort>
  .key_of(rel, row) -> Value
```

Preconditions: `program` is `check_schema(schema)`; `t` is a typed
kernel term of `terms`, an arena `adtb-lang` returned for `schema`
([kernel.md](kernel.md)), holding no write, block or `fresh`; `params`
binds each free variable of `t` to a value of its type; `db` is a
database as defined below.

Postconditions:

- `Ok(v)`: `v` is the denotation of `t` over `db` by the equations of
  [../spec/semantics.md §5](../spec/semantics.md#5-queries). A final
  result ordered by `order by` is a `Value::List`, in the order
  [§5.9](../spec/semantics.md#59-top-n-and-presentation) states.
- `Err(Abort::Fault(f))`: the denotation is the fault `f`. Inside the
  term, `try` turns `f` into `Err` of the prelude `Fault` constructor
  named by `f`.
- `Err(Abort::Limit(Limit::Rounds { rounds }))`: a `fix` took more
  steps than its limit of `rounds`; the statement is rejected with
  `StatementLimit { limit: Rounds { rounds } }`.
- `Err(Abort::NotYet(form))`: the term holds a form the interpreter
  does not evaluate yet: `at`, a Wasm or host function, or a Parquet
  or Arrow IPC file scan.
- `Err(Abort::Internal(what))`: evaluation reached a value or term the
  preconditions rule out, such as `Unreachable`, a primitive applied to
  operands of another type, or a `case` with no arm for its
  scrutinee's constructor; `what` names the invariant. The statement
  is rejected with `Internal { what }`. `try` does not catch it, and a
  checked primitive does not turn it into `None`.

A CSV or NDJSON file scan reads the files `with_files` gives, a
`Files` that returns the files a path names, by name in name order,
with their contents; without one it reads the file system, a relative
path against the working directory. `Disk::new(root)` reads the file
system with relative paths against `root`, and `InMemory` holds files
by name. `Files`, `Disk` and `InMemory` are those of `adtb-file`, whose
`records` returns each record of the files a path names, in order, with
its file, its number, its text as the file holds it, and its row or why
it does not convert; the engine reads files through them. A component of a path holding `*` or `?` matches every name it
matches. A file that cannot be read, or whose text is not UTF-8, is
`BadInput` naming it with line 0. Records are numbered from 1 across
every file the path names, in name order. A file scan does not pin the
files it reads across statements.

`caller` denotes the value given by `with_caller`. In a constraint
body, `inserted`, `deleted` and `prior` read the transition given by
`with_change`, and the empty change without one.

`apply` evaluates `t`, a function of its arguments such as a
registered query's `Lam` chain, and applies it to `args` in order, or
to `()` when there are none; its preconditions and postconditions are
those of `eval` for the application. `key_of(rel, row)`, for `row` a
row of `rel`, returns the value `key` of that row denotes.

### 1.1 Statements

```text
Interpreter::new(schema, program, db)
  .run(terms, t, params, &mut supply) -> Result<Outcome, Abort>
  .run_call(terms, t, args, &mut supply) -> Result<Outcome, Abort>
Outcome { result: Value, change: Change, db: Database }
```

`run` evaluates one statement against the snapshot `db` by
[../spec/semantics.md §6.6](../spec/semantics.md#66-statements): `t` is
a `Transact`, a write (a term of type `Change`, an item of ad hoc
text), or a query, whose outcome changes nothing. `run_call` does the
same for a registered transaction, a `Lam` chain over its parameters
(kernel.md §4.3), applied to `args` in order, or to `()` when it has
none. `run` also accepts a normal form (kernel.md §7), a registered
transaction's with `params` binding its parameter variables.

Preconditions: those of `eval`, except that `t` may hold writes, blocks
and `fresh`; `db` is valid (every constraint of the schema holds in
it).

Postconditions:

- `Ok(o)`: the statement succeeds. `o.result` is the block's result,
  `()` for a write. `o.change.prior` is `db`, and `o.change.inserted` and
  `o.change.deleted` are the normalised change set `Δ*`, with every
  delete action derived. `o.db` is `apply(db, Δ*)`, which holds no empty
  constructor relation.
- `Err(Abort::Reject(r))`: the statement is rejected with the
  `RejectReason` constructor `r` names: `UserError` for `fail`,
  `KeyChanged`, `BadInput`, `NotOpen`, `RowTooLarge`, `KeyViolation`
  with the key in the textual syntax, or `Violated` with the
  constraint's canonical name and every row its query returns. Of several violations, the least name is
  reported. Relation names are unqualified, constraint names qualified
  by module.
- `Err(Abort::Fault(f))`: a write, a delete action's fill or a
  constraint faulted. A fault in a constraint is the outcome whatever
  else the statement violates.
- On every `Err`, nothing is applied.

`with_row_limit(measure)` gives the size of a row's stored form and the
limit its constructor's rows must fit, `measure(relation, ctor, row)`
returning `(size, limit)` in bytes. Each new row of `Δ*` is measured
after `NotOpen` and before keys and constraints; the first, by relation,
constructor and row, whose size is over its limit is `RowTooLarge`.
Without a measure no row is too large.

Constraint checking is naive: every key, unique, reference, domain,
check and declared constraint of the schema is evaluated over the
whole transition at every statement boundary.

A constructor relation or value-level constructor whose stage in the
schema ([kernel.md §6](kernel.md#6-the-schema-a-term-refers-to)) is
`Declared` or `Deprecated` refuses writes. `with_closed` closes more
constructors, each by its sum and constructor index with its stage,
for every relation of the sum and as a value; it never opens one the
schema closes.

`supply` is the context's supply of generated values, kept by the
caller across statements:

- `supply.clock` is the instant every `generate clock` fill of the next
  statement takes;
- a `sequence` or `uuidv7` value differs from every value `supply` has
  drawn for the relation, in statements that failed included, and from
  every key of the relation in the snapshot of every statement `supply`
  has served;
- each statement that inserts into a rejects relation draws one `load`
  value, greater than every `load` it drew before and every `load` in
  that relation.

Within one run, a write term is evaluated as often as the term
structure says, and each evaluation of a `fresh` draws a new value
(kernel.md §4.1). A node shared by several parents is evaluated once
per parent.

The interpreter shares nothing with the engine except the typed kernel
terms of `adtb-lang`: no indexes, no plans, no pages.
It answers what any generated query over any generated database should
return.

### 1.2 Textual syntax

```text
textual::print(schema, v, ty) -> String
textual::parse(schema, text, ty) -> Result<Value, ParseError>
textual::read_document(schema, text) -> Result<Document, ParseError>
textual::print_document(schema, doc) -> String
Document { header: Option<Header>, relation: RelId, rows: BTreeSet<Value> }
Header { edition: String, version: u64, hash: String }
```

These read and print the textual syntax of
[../spec/interop.md §1](../spec/interop.md#1-textual-value-syntax)
over the values of §2, at the reader's version of `schema`. Each is
the function of [kernel.md §9](kernel.md#9-textual-syntax) at
`Form::of(schema, ty)`, or at `schema` for a document, over `Value`,
and has its preconditions and postconditions.

### 1.3 Deviations

- `matches` takes its Unicode classes (`\p{..}`, `\w`, `\d`, case
  folding under `i`) from the tables of `regex-syntax`, and every other
  text function from the ICU4X data. The two need not follow one
  Unicode version, so where they differ `matches` does not follow the
  database's recorded version
  ([../spec/model.md §3.5](../spec/model.md#35-text)).

## 2. The database map

A database is a map from (relation variable, constructor index) to a
set of rows, so two relation variables of one type stay apart. A
constructor index is the kernel's: the constructor's position in its
sum's declaration. A row is a `Value::Ctor` of that index holding every
field, header fields first, each in declaration order; a key field
holds a `Value::Key`. A sum is the union of its constructor relations,
a `match` the union of its arms, a change set the inserted and deleted
relations per constructor.

## 3. Comparison rules

A harness compares an engine result with the interpreter's as sets,
with nested results compared as nested sets. Float results are compared
within the bound of
[../spec/semantics.md §1.3](../spec/semantics.md#13-conformance);
every other value is compared exactly.

Rulings: project/oracle/rust-reference, project/oracle/shared-front/typed-input, project/oracle/float/tolerance

## 4. Generator interface

Inputs are generated well-typed, in three ways:

| Method | What it generates |
| --- | --- |
| Type-directed | A sum, its constructors and fields, rows per constructor heap, then a query built rule by rule from the typing rules, choosing `match` arms from the scrutinee's constructor set; failures shrink to a small schema and query |
| Corpus mutation | Subterms of the conformance suite and registered bundles mutated, keeping the mutants that still type-check |
| Schema walk | Random sequences of schema operators applied to a populated database, interleaved with writes and queries, re-registering bundles after each; the reference interpreter checks every query before and after |

Postcondition: every generated term type-checks, so the interpreter
and the engine both accept it.

A generator gives the harness cases, each a function of its seed and
its number of queries:

```text
case(seed, n) -> Case { schema, migration, load, queries: [Query; n], tolerance }
query.text()         -> text
query.by_ctor()      -> Option<[text]>
query.by_predicate() -> Option<Split { whole, parts: [text; 3] }>
query.counts()       -> Option<Split { whole, parts: [text; 3] }>
```

Postconditions:

- `schema` is the schema file of module `app`; it builds and checks.
  `migration` adds module `app` with the same declarations, and an empty
  engine database accepts it.
- `load` is a sequence of ad hoc writes. Applied in order from the empty
  database, each succeeds on the interpreter and on the engine, so both
  hold one database.
- Each `query.text()` is ad hoc text of one query over `schema`, holding
  no write, `order by` or parameter. It may fault. A sum of floats in it
  reaches only its result: no comparison, arithmetic, key or group reads
  one.
- `tolerance` is at least the bound of the comparison rules for every
  sum of floats a query of the case computes. Two results agree when
  they are equal but for floats that differ by at most `tolerance`.
- `query.by_ctor()`, when the query's first generator ranges over a sum
  relation and the query does not group, holds one query per
  constructor that generator ranges over, the generator ranging over
  that constructor's relation alone. The union of their results is the
  result of `query.text()`.
- `query.by_predicate()`, when the query does not group, holds a
  predicate `p` of the query's bindings as three restrictions of
  `whole`, the query without `p`: to the bindings where `p` is true,
  false, and where it faults, each tested through `try`. The union of
  their results is the result of `whole`.
- `query.counts()`, when the query is a comprehension the generator
  drew, holds `whole` as a `count` over the bindings of the query and
  `parts` as its three restrictions by `p`; `whole` is the sum of the
  parts.
- Where one side of a law faults, the other faults too. Which fault is
  reported is not part of a law.

Every method of the table gives cases of this shape. A corpus case's
queries are the queries of one conformance script of one module and
their mutants that check; they have no partitions and no counts, and a
script whose results hold generated keys or clock values other than a
sequence's makes no case. A schema walk gives a case before its first
migration and two per migration after it:

```text
walk(seed, steps, n) -> Walk { start: Case, steps: [Step; steps] }
Step { migration, operators: [Operator], inverse: Inverse, migrated: Case, writes: [text], written: Case }
Inverse = Restores { revert, reverted: Case } | Refused { revert }
```

Postconditions:

- `start.migration` followed by the `migration` of each step, in order,
  is a log the engine applies from the empty database; the migration of
  a step applies to the database `start.load` and the writes of the
  steps before it leave.
- `migrated` is that database once the step's migration applies,
  stated as a case of the schema the migration leaves: its `load`
  builds the same rows from the empty database. Its queries range over
  that schema.
- `writes` succeed on the engine after the migration and on the
  interpreter holding `migrated`; `written` is the database they leave,
  with the queries of `migrated`.
- No operator of a migration names what an earlier operator of the same
  migration creates, but for the type a merge or a nest declares before
  it and the enable of a constructor added before it.
- `operators` are the operators of the migration, in order, each a kind
  the walk can draw; a walk drawn with a set of kinds to skip holds none
  of them.
- `inverse.revert` is a migration `revert n` of this step's migration,
  with a `fill` for each value a lossy operator of it discards. Applied
  right after the step's migration, it is refused when `inverse` is
  `Refused`, which holds exactly when an operator of the migration
  discards data and has no inverse. Otherwise it applies and leaves
  `reverted`: the database before the migration, but for each value a
  lossy operator discarded, which holds that operator's fill, with the
  queries of the case before the migration.

Rulings: project/testing/gen/type-directed, project/testing/gen/corpus-mutation, project/testing/gen/schema-walk

## 5. Notes for the implementation

Non-binding.

### 5.1 What the oracle tests

The interpreter and the engine both consume generated typed kernel
terms, so differential tests check planning and execution, not the
front end. The front end is tested on its own:

- Desugaring: a surface term and its desugaring give equal results in
  the interpreter.
- Typing: a well-typed term never fails at run time with a type error.
- Fuzzing (§5.6): malformed input is refused with a typed error, never
  a crash.

### 5.2 Differential testing

A seed generates a schema, a database and a query; the engine's result
must equal the reference interpreter's, as sets, with nested results
compared as nested sets.

```text
seed -> (schema S, database D, query q)
assert engine(q, D) == reference(q, D)
```

### 5.3 Constructor partitioning

A sum is the disjoint union of its constructor relations, so a query
over the sum equals the union of the same query over each constructor:

```adt
from i in identities where p(i) select f(i)
  ==  (from i in identities.EmailPass where p(i) select f(i))
    union (from i in identities.Google where p(i) select f(i))
    union (from i in identities.Apple where p(i) select f(i))
```

The left side runs through the key index and merged heap scans; the
right side through single-heap scans and constructor pruning. The same
holds for each `match` and the union of its arms. This checks the
planner without the reference interpreter.

### 5.4 Predicate partitioning

A query equals the union of its restrictions to `p` and to `not p`.
There is no NULL and `Bool` has two constructors, so two partitions
cover every row that `p` does not fault on; a third, `try p` returning
`Err`, covers the rest. Aggregates decompose the same way:

```adt
count(x for x in g) == count(x for x in g where p) + count(x for x in g where not p)
```

### 5.5 Plan toggles

Every query gives the same result with any subset of the planner's
rewrites disabled: arm distribution of joins, constructor pruning,
probe fusion, and compiled versus interpreted pipelines.

```text
for flags in subsets(rewrites): assert run(q, D, flags) == run(q, D, none)
```

### 5.6 Front-end fuzzing

Coverage-guided fuzzing drives the parser, the type checker and the
wire decoder with arbitrary bytes. Every input is either accepted or
refused with a typed error; none crashes.

### 5.7 Schema operator laws

For every schema operator with an inverse, `inv(op)(op(D)) == D`; for
an operator with a declared loss, the law holds on the declared
projection. For a query `q` pinned to version `v` and its translation
`q'` at the version `op` produces, `q(D) == q'(op(D))`.

The example starts from a version of a local schema in which `Login`
has a constructor `Social`:

```adt
type Provider = Google | Apple
type Login { id: Key logins } = Password { hash: Bytes }
                              | Social   { provider: Provider, sub: Text }
relation logins : Login key id generate sequence

-- 0002
split constructor Login.Social into Google { gid: Text } | Apple { sub: Text }
  by match
    | Social { provider: Google, sub } -> Google { gid: sub }
    | Social { provider: Apple, sub }  -> Apple { sub: sub }
-- law: the inverse merge, keeping the tag as provider, restores D
```

### 5.8 Schema walks

A schema walk (§4) exercises recompilation of registered queries, lazy
page format rewrites and staged schema jobs.

Rulings: project/testing/differential, project/testing/ctor-partition, project/testing/tlp, project/testing/plan-toggle, project/testing/front-fuzz, project/testing/smo-laws

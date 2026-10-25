# Oracle

`adtb-reference` is a naive evaluator of the kernel calculus of
[../spec/semantics.md](../spec/semantics.md) over in-memory sets. It is
the oracle of the test harness in `adtb-sim` and of the conformance
runner.

## 1. Evaluation contract

```text
fn eval(q: &Kernel, db: &BTreeMap<(RelVarId, CtorId), BTreeSet<Row>>) -> BTreeSet<Value>
```

Preconditions: `q` is a typed kernel term of `adtb-lang`
([kernel.md](kernel.md)), and `db` is a database as defined below.

Postcondition: the result is the denotation of `q` over `db` by the
equations of [../spec/semantics.md §5](../spec/semantics.md#5-queries).

The interpreter shares nothing with the engine except the typed kernel
terms of `adtb-lang`: no indexes, no plans, no pages.
It answers what any generated query over any generated database should
return.

## 2. The database map

A database is a map from (relation variable, constructor id) to a set
of rows, so two relation variables of one type stay apart. A sum is the
union of its constructor relations, a `match` the union of its arms, a
change set the inserted and deleted relations per constructor.

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

TODO: the interface a generator exposes to the harness is undecided.

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
cover every row. Aggregates decompose the same way:

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

# Schema changes

Evolution applies schema operators ([../spec/evolution.md](../spec/evolution.md))
as writes. The catalog stores the result and the planner replans
against it.

## 1. The catalog is data

The catalog is a set of relations in the engine's own model, read by
the query language and written by ordinary transactions
([../spec/catalog.md](../spec/catalog.md)).

Invariant: every transaction sees the catalog of its snapshot, so a
schema change is a write like any other.

A schema operator runs in a transaction and commits or rolls back with
the data writes beside it.

Every relation of the `catalog` and `sys` modules, and every relation
a migration declares, is stored as one heap per constructor, with an
index for its key and for each of its uniques and indexes. The file
header names the catalog's root page
([storage.md §1.2](storage.md#12-file-header)), and the root page names
the commit log's root and the heap that holds the root of every other
heap and index, the order key of every constructor of a declared sum
and the text of every migration.

```text
Catalog::create(rt, &pager) -> Result<(Catalog, Transactions), Error>
Catalog::open(rt, &pager)   -> Result<(Catalog, Transactions), Error>
catalog.migrate(&pager, &txns, text) -> Result<(), Error>
catalog.rows(&pager, &txns, &snapshot, relation) -> Result<[(ctor, row)], PageError>
catalog.schema() -> Schema
catalog.source() -> evolve::Source
catalog.order_keys() -> order key by constructor id
catalog.plan() -> plan::Catalog
catalog.inventory(&commit_log) -> verify::Inventory
catalog::declared(&schema, &source) -> rows by relation
```

Preconditions:

- `create` is given the pager of a new file with nothing allocated.
- `open` is given a recovered pager.

Postconditions:

- After `create` returns, the relations of `catalog` hold, as one
  committed transaction, the rows `declared` gives for the engine's
  own declarations: a row of `types`, `constructors`, `fields`,
  `relations` and `indexes` for each such object with a catalog id,
  keyed by that id; each index's constructor set in `index_ctors`; and
  every write state in `ctor_states` and `value_states`. Every other
  relation is empty.
- `create` sets the file header's catalog root before its transaction
  begins, and names the roots heap in the root page once that
  transaction commits. `open` of a file whose header names no catalog
  frees every page and creates the catalog again; `open` of one whose
  root page names no roots heap frees every page but the commit log's
  and the root page, and creates the catalog again with the same commit
  log, so no transaction id is assigned twice.
- `migrate` applies the first migration of the file, holding only
  operators that declare objects, as one transaction: it creates the
  heaps and indexes of every relation it declares, writes the rows
  `declared` gives for every object it declares, and returns once the
  migration is durable. A refused migration changes nothing.
- After a crash during `migrate`, `open` finds either the whole
  migration or none of it, and frees every page the migration
  allocated if it did not commit.
- `open` finds every heap and index from the file header alone, and
  the schema by folding the stored migrations from the empty schema.
  Heap ids and index order keys are those stored, never computed again.
- `rows` returns the rows a snapshot sees.
- `plan` describes every relation: a heap for each constructor, no two
  with one heap id, and each index by its catalog id, the key index
  among them.
- `inventory` names every page the catalog allocated, so `verify` over
  it after `create`, `migrate` or `open` reports nothing.

Rulings: engine/catalog-relations, engine/transactional-ddl

## 2. Staged states

Operators that need intermediate states pass through them as staged
states, each compatible with the last
([../spec/evolution.md §3.4](../spec/evolution.md#34-staged-changes)):

| Object | States |
| --- | --- |
| constructor added by `add constructor` | declared (readers can type it, no writes) then writable, then deprecated, then removed when empty; `disable` returns a writable constructor with no rows to declared |
| constructor of a value-level sum added by `add constructor` | the same states; while declared, no field of any relation stores a value of it |
| constructor created by `add type` | writable from the start |
| product of split, merge, nest or unnest | writable when every source constructor is writable |
| index | write-only, backfilled, readable |

The write state of each constructor is a row of `catalog::ctor_states`,
keyed by (relation variable, constructor).

Invariant: each state is committed and seen by every running
transaction before the next begins.

The user's transaction commits the first state; a background job runs
the rest and records its phase in the catalog.

Postconditions of a migration with staged jobs:

- The first state is one no query at version `n - 1` can observe; the
  migration is pending from its commit.
- The migration's schema version commits only when every staged job
  of the migration completes. The last state commits version `n`
  together with the logical effect of every operator of the migration
  and its `script` writes, computed at that commit's snapshot; only
  then is the migration appended to the log.
- A failed job discards the pending migration: the operator log is
  unchanged, the physical states the job committed are removed, the
  report names the operator and the rows that caused the failure, and
  no inverse operators are appended.
- Every check the migration's operators made when they applied reruns
  at the commit of its last staged job, and a failure discards the
  pending migration as a failed job does.

A host function must be bound before any staged job or plan needs it
([../spec/embedding.md §10](../spec/embedding.md#10-host-functions)).

Registered queries pinned to an older schema version keep running
through translation by the operators' inverses while the staged change
rolls out ([../spec/evolution.md §7](../spec/evolution.md#7-version-translation)).

A query `q` registered at version `n` and called at version `m` is
translated once into a query `q'` over `Σ_m`. Postcondition: the
translation is correct when `⟦q'⟧_{Σ_m}(D_m) = ⟦q⟧^{n}(D_m)` for every
valid `D_m`, with `⟦q⟧^{n}` as
[../spec/semantics.md §9.6](../spec/semantics.md#96-version-translation)
defines it.

TODO: the contract of the recheck pipeline for registered queries is undecided.

Rulings: engine/online-states, engine/ddl-online-txn/staged-job, engine/pinned-version

## 3. Log records

Schema operators and stage transitions are log records
([storage.md §5.1](storage.md#51-records)), applied in log order with
the data on recovery and on replicas.

Invariant: a replica applies the operator that declares a constructor
before any row of it, and a stage transition before any change that
depends on it.

A migration whose operators need staged jobs writes the record naming
its schema version only when its last staged job completes; a failed
job discards the pending migration, and the operator log is unchanged.

Applying a migration writes the schema version; the transactions layer
serializes it against the transactions that ran at the previous version
([mvcc.md §8.2](mvcc.md#82-siread-locks)).

Rulings: repl/schema/in-log

## 4. The schema transition

`adtb-lang` provides the type-level transition of a migration
(`evolve.rs`): evolution hands it the schema before a migration and
the migration's text, and receives the schema after it or the reasons
it is refused. The checks of
[../spec/evolution.md §3.2](../spec/evolution.md#32-checks-at-apply)
that read data, bundles, leases or grants of roles are evolution's,
not this function's.

| Entry point | Returns |
| --- | --- |
| `Source::new(decls)` | the source of a schema declared directly, every constructor relation and value-level constructor `Enabled` at version 0 |
| `Source::migrate(m, text)` | the source after migration `m`, whose spans index `text`, numbered `version + 1` and appended to `log`; or a `Refusal` |
| `Source::check()` | the built `Schema` and its `Program` ([kernel.md](kernel.md)) |

A `Source` holds the declaration texts of every user module, the
modules that hold nothing, the installed extensions, the write state
of each (relation variable, constructor) and of each value-level
(type, constructor) with the version that set it, the catalog id of
each object with the next id of each space, and the log of the
migrations applied to it with the source before each. `Source::new`
numbers its objects as `schema::build` does.

Preconditions of `migrate`:

- `m` parses as a migration file and its operators' spans index `text`.
- `self.check()` succeeds.

Postconditions of `migrate`:

- Each operator applies to the source the operators before it in `m`
  produce. A refused operator leaves that source unchanged; the others
  still apply, so the `Refusal` names every failing operator, each at
  its span.
- On success the result checks: `result.check()` succeeds. Otherwise
  `Refusal.recheck` names every schema object of the result that fails
  to build or check, and `Refusal.result` is that result.
- The result's declarations are the operators' rewrite of the source's
  text. A declaration no operator touches keeps its text.
- Every object keeps its catalog id across the migration, a renamed
  one included. An object the migration creates takes the next id of
  its space, in the order its operators create objects, and an id is
  never given twice. A unique or index whose fields or constructors a
  split, merge, nest or unnest rewrites, and a field `move field`
  moves, is a new object.
- `result.schema()` carries the result's ids and write states
  ([kernel.md §6](kernel.md#6-the-schema-a-term-refers-to)).

Every operator resolves the names it writes in the migration's
module, or the module that qualifies them. Each row states what the
operator requires of the source before it, beyond its names resolving,
and what holds after it. "Its relations" are the relation variables
whose element type is the operator's type.

| Operator | Precondition | Postcondition |
| --- | --- | --- |
| `add module m` | `m` is not `prelude`, `catalog` or `sys` and is no module | `m` is a module with nothing in it |
| `drop module m` | `m` holds no declaration | `m` is no module |
| `install m ...` | as `add module` | `m` is a module and an installed extension |
| `uninstall m` | `m` is installed; no declaration outside `m` names `m::` | `m` and its declarations are gone |
| `add type`, `add relation`, `add set`, `define fn`, `define aggregate`, `add constraint`, `add check`, `store` | the module exists and declares nothing of that kind and name; a relation name is unqualified | the declaration is the text after the verb; a type's constructors are `Enabled` value-level; a relation's constructors take their value-level state, else `Enabled` |
| `redefine fn` | the function exists | its declaration is the new text |
| `drop type T` | no relation variable, field, function signature or body names `T` | `T` and its value-level states are gone |
| `drop relation r` | no field outside `r`'s own type references `r`; no body names `r` | `r`, its named sets and its states are gone |
| `drop fn`, `drop aggregate`, `drop constraint`, `drop check`, `unstore`, `drop set` | the object exists | it is gone; dependents are left to the recheck |
| `rename type`, `relation`, `set`, `constructor`, `field` | the new name is free in its scope: the module, the relation variable's constructors and sets, or the constructor and header | every declaration that names the object names it by the new name, a punned field `{ f }` becoming `{ g: f }`; write states follow the name |
| `add constructor T.C` | `T` is a sum, a record included, not a newtype; `C` is no constructor of `T` and no named set of its relations; a `before`/`after` constructor exists; each `joins r.S` names a named set of one of its relations; each `arm` target has exactly one match whose scrutinee holds `C` at the new version | a record `T = { ... }` becomes `T = T { ... } \| C`; `C` is in `T` at its place; each joined set holds it; each arm ends its match; `C` is `Declared` in each of its relations and value-level |
| `remove constructor T.C` | `C` is `Deprecated` in each of its relations (value-level, when it has none) since an earlier migration; no reference type names a constructor set holding `C` but the whole sum; no named set is `C` alone; no check ranges over `C` alone | `C` is gone from `T`, every named set and listed scope; a unique, index or default over `C` alone is gone; constraints lose their arms for `C`; its states are gone |
| `enable`, `disable`, `deprecate` | `r.C` or `T.C` names one constructor; `enable` meets one not `Enabled`; `disable` and `deprecate` meet one `Enabled` | its state is `Enabled`, `Declared` or `Deprecated` since this version |
| `split constructor T.C into ...` | the products are no constructors of `T`; the routing is a match over rows of `C`, exhaustive, reading only the row, each arm a product's value, omitted header fields carried; a unique or index over `C` alone with a field no product keeps has replacements stated | the products replace `C` at its place; a scope over `C` alone moves to each product that keeps all its fields, under their new names, and the stated replacements are added; listed scopes, named sets and reference types over `C` list the products; arm clauses replace `C`'s arms; products take `C`'s states |
| `merge constructor T.(C \| D) into M` | `M` is no constructor of `T`; `M` has exactly one discriminator field; with `by`, the routing is as a split's and gives the discriminator as each row's constructor; without it, fields match by name and type; no source is `Deprecated`; no named set, scope or reference type holds some but not all sources; no check ranges over some but not all | `M` replaces the sources at the first one's place; a unique over sources becomes a unique over `M` on the discriminator and the fields the routing carries; sets and scopes holding all sources hold `M`; arm clauses replace the sources' arms; `M` is `Declared` where a source is, else `Enabled` |
| `nest constructor T.(C \| D) into N { f: S }` | `N` has one field, typed by a sum `S` whose constructors have the sources' names and fields; no source is `Deprecated`; scopes as for a merge | `N` replaces the sources; a scope over one source's field `g` becomes `f.C.g` in `N`; states and arms as for a merge |
| `unnest constructor T.N` | `N` has one field of a sum with no header or parameters, whose constructor names are free in `T` | `S`'s constructors replace `N`, as `S` writes them; a field `f.C.g` of a scope over `N` becomes `g` in `C`; states and arms as for a split |
| `add field T.C.f: U fill e` | `f` is no field of `C` or the header (of any constructor, for a header field); `e` checks at `U` over a row of `C`, reading only the row | `f: U` ends the field list |
| `drop field T.C.f` | the field exists | it is gone |
| `move field T.C.f to T fill e` | no other constructor has `f`; `e` is a match over the other constructors' rows at `f`'s type | `f` is a header field and gone from `C` |
| `move field T.f to T.(C \| D)` | `f` is a header field; no fill | each listed constructor has `f`; the header does not |
| `add unique`, `add index`, `add interval index` | none beyond its fields checking | the clause ends the relation's declaration |
| `drop unique`, `drop index` | a clause of that kind with the same fields, token for token, and the same constructor set exists | it is gone |
| `rekey relation r key (...) carry ...` | the carried fields are exactly the fields outside `r`'s own type typed `Key r` or `Key r.S`; at the end of the migration no field of `r`'s own type typed `Key r` lies outside the new key | `r`'s key clause is the new key, with no generator |
| `add default`, `drop default`, `add computed`, `redefine computed`, `drop computed` | the default or computed field exists, for a drop or redefine | the clause is added, replaced or gone |
| `grant`, `revoke` | each relation names a relation variable of a user module; each function a schema function; no capability or bundle call privilege | the source is unchanged |
| `adopt tzdb` | none | the source is unchanged |
| `script w` | `w` checks as ad hoc text against the schema the operators before it produce | the source is unchanged |
| `revert n` | `n` is in the log and holds no `script`; no later migration names an object `n` creates; each inverse that adds a field the log holds no value for has a `fill` | the inverses of `n`'s operators, last first, apply as the table states, computed against the source before each; an added constructor goes with its arms, a removed one comes back at its place with its states and its constraints' arms, a split or merge is inverted when its routing binds or fixes every field and carries each binding whole, else refused |

Write states are the source's, not the catalog's: evolution writes
`catalog::ctor_states` and `catalog::value_states` from them. A
refusal of evolution's own checks (data, bundles, readers-first) is
evolution's to report.

### 4.1 Names of operands

The catalog writes each operand written in the language as a `Term`
([../spec/syntax.md §2.9](../spec/syntax.md#29-canonical-token-form)).
`adtb-lang` provides the resolution of its names and the spelling:

| Entry point | Returns |
| --- | --- |
| `Source::names()` | for each declaration `i`, the `Names` of file `i` of `Source::files()`, by span |
| `Logged::names()` | the `Names` of the migration's text, by span; or the operands whose names it cannot resolve |
| `canon::canonical_term(text, span, names)` | the canonical token form of the term at `span` of `text` |

Postconditions of `Source::names()` and `Logged::names()`:

- A `Names` holds, at the span of each identifier that names a type,
  constructor, named set, relation variable, function, aggregate,
  constraint or check, the module of the object it resolves to: the
  object it denotes where it is checked, a constructor resolved by its
  expected type included. A declared name resolves to the declaring
  module.
- `Logged::names()` resolves an operand written into a declaration
  (a declaration, a constructor, a field type, an arm, a unique or index
  field, a default, a computed field) as it resolves in that
  declaration of the source after its operator, or of the migration's
  result when that source does not build; the fields of a dropped
  unique or index as in the clause before the operator; a fill or a
  routing as its operator checks it. It holds no names for a `script`
  write, a grant or revoke target, or a `revert` fill.

Postcondition of `canonical_term`: a name with a module is spelled
`module::name`, a qualifier written before it dropped; a name of the
prelude, and every other token, as the canonical token form states.
Precondition: `span` lies on token boundaries and `names` resolves
every name in it.

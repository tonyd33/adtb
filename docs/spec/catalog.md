# Catalog and operations

This part defines the relations the engine provides about itself: the
catalog of schema objects, the volatile statistics and activity
relations, and their types. It defines the prelude error sums, plan
inspection, resource control, the operational commands and the command
line tool, configuration, integrity verification and maintenance, the
compatibility promises across engine releases, and telemetry export.
How catalog sums change across engine releases is defined in
[evolution.md](evolution.md#10-engine-releases); who may read the
catalog is defined in [security.md](security.md#7-catalog-visibility).

Examples use the running schema of [README.md](README.md#conventions)
at version 12, after migration 0011 of
[evolution.md](evolution.md#42-add-a-constructor) adds the constructor
`Apple { sub: Text }` to `Identity` and joins it to `identities.OAuth`,
and migration 0012 enables `identities.Apple`. Each section's examples are an
independent history starting from that state unless the section states
another version.

## 1. System relations

### 1.1 Two modules

The engine provides two modules of relations.

- `catalog::` holds the schema and the engine's other transactional
  metadata. Its relations are relation variables in the sense of
  [model.md](model.md#6-relation-variables): every read is part of the
  reading transaction's serializable history, and a transaction sees
  the catalog of its snapshot.
- `sys::` holds statistics, activity and process state. Every relation
  in it is volatile (§1.3).

Both are queried with the query language, by the same comprehensions,
matches and aggregates as user relations. No other interface to this
information is normative: the command line tool (§9.4), the telemetry
export (§13) and the in-process metrics callback present the same
facts.

A program must not insert into, update or delete from a relation of
either module. Catalog relations change only through schema operators
([evolution.md](evolution.md#4-operators)), registration and other
deployment state ([applications.md](applications.md#41-registration)),
role, grant and peer mapping statements ([security.md](security.md#3-grants)), and
the commands of §9, including `set setting` (§10.2). A write naming a
system relation is refused with `Rejected { reason: Denied {..} }`.

A bundle that reads `catalog::` or `sys::` relations pins a catalog
version in its header beside its schema version, and is served at that
catalog version ([evolution.md](evolution.md#10-engine-releases)).

```adt
from c in catalog::constructors, s in catalog::ctor_states
where c.sum.name == "Identity" and s.ctor == key c
  and s.relation.name == "identities" and s.stage == Enabled
select { name: c.name, id: key c }
```

### 1.2 Identity of catalog objects

Every catalog relation whose key is one field has a key field typed
`Key catalog::r` for its own relation, filled by the engine, except
`catalog::wasm_modules`, keyed by the hash of a module's bytes (§2.3).
The key value of a row of `catalog::types`, `catalog::constructors` or
`catalog::relations` is the object's id.

`r.key(v)` of a `catalog::` or `sys::` relation takes the relation's
declared key type: an `I64` for a key field typed `Key r` of its own
relation, which holds a catalog id or another engine-assigned number,
and the field's type otherwise, `Bytes` for `catalog::wasm_modules`:

```adt
-- given h: Bytes
from c in catalog::constructors where key c == catalog::constructors.key(1052) select c.name
from w in catalog::wasm_modules where key w == catalog::wasm_modules.key(h) select w.bytes
```

Every schema object has a catalog id, the key of its row: types,
constructors, relation variables, fields, named sets, indexes and
uniques, constraints and checks, functions and aggregates, defaults
and computed fields. A stored view is a relation variable of kind
`Stored` and has its id as one. Operators name every object by its
catalog id (§2.4).

Ids are a deterministic function of the migration log: the same log
gives every object the same id, and every tool replaying the log
computes the ids the database holds. Types, constructors and relation
variables share one id space; every other kind of object has an id
space of its own. In every space the log's ids start at 1024, and each
created object takes the next id of its space. Objects are created in
log order: by migration, then by operator in ordinal order, then
within an operator in this order:

| Operator | Objects created, in order |
| --- | --- |
| `add type` | the type, then its constructors in listed order (a record or newtype has one); the header fields in declared order, then each constructor's fields, constructors in listed order and fields in declared order |
| `add relation` | the relation variable; then, for its clauses in written order, an index for the key and for each `unique` and `index`, a default for the key's `generate` and, for each `default`, one for each field it fills (the header's field, or each constructor's field in declared order), and a computed field for each `computed` |
| `add set` | the named set |
| `add constructor` | the constructor, then its fields in declared order |
| `split`, `merge`, `nest`, `unnest` | the new constructors in listed order (for `unnest`, in the order of the field's sum), each followed by its fields in declared order; then, for each relation variable of the type in id order, an index for each unique and index the operator re-scopes or a split's `unique` and `index` clauses add, in the order of the relation variable's clauses after the operator |
| `add field` | the field |
| `move field` | for a move down, a field for each listed constructor but the first, in declaration order |
| `add unique`, `add index`, `add interval index` | the index |
| `rekey` | an index for the new key |
| `add default` | the default, one for each field it fills, as for a `default` clause |
| `define fn`, `define aggregate` | the function |
| `add constraint`, `add check` | the constraint |
| `store f` | the relation variable of the view; then, when the view's element type is anonymous, a type for it with one constructor, then that constructor's fields when the element type is a record, in the order the function's result type writes them; then an index for its key and for each `unique` and `index` clause, in written order |
| `install` | what each declaration of the package creates, as the `add` operator that declares it, in package order |
| `add computed` | the computed field |
| `revert` | what each of its inverse operators creates, as that operator |

```adt fragment
-- 0101, from a version 100 of a separate history, before Apple,
-- at which the next shared id is 1050 and the next field id 2010
module auth
add type Device = { id: Key devices, owner: Key users, label: Text }
-- Device 1050, its constructor 1051; fields id 2010, owner 2011, label 2012
add constructor Identity.Apple { sub: Text }
-- Apple 1052; field sub 2013
```

An operator records the id of the first object it creates (§2.4); the
ids of the others follow from this order and the operator's `decl`
term.

Renaming, redefining, enabling, disabling or deprecating an object
leaves its id unchanged. A constructor re-added by an inverse is a new
constructor and takes a new id. A unique or index that a split, merge,
nest or unnest re-scopes, one whose clause names a constructor the
operator replaces, is a new index with a new id, and the old index is
dropped. An index with no `in` is not re-scoped and keeps its id. A
stored view of a declared type `T` is a relation of `T`, and `store f`
creates no type for it. A field that `move field` moves keeps its id: moved up, the
header field has the id of the constructor's field; moved down, the
field of the first listed constructor in declaration order has the
header field's id, and the field of each other listed constructor is
new. Ids below 1024 are those of the fixed
table of §4.6. A constructor's id is its wire tag and its stored tag.
An id belongs to the declared constructor of a type and is shared by
every instantiation of a parametric sum. Key values of every catalog
relation are never reused.

A catalog row naming a constructor, relation, index, constraint,
function, default, computed field or bundle is never deleted. Removing or dropping the object
records that in the row (a constructor, its fields, named sets and the
other objects get `dropped: Some { value: v }` with
the schema version `v` that dropped them). A key held in a `sys::` row or in a stored plan
therefore always resolves.

Rulings: spec/ids/operator-ids/implied, spec/ids/rescoped-index/new, spec/ids/rescope/named, spec/ids/moved-field/keeps, spec/ids/move-down/first-declared, spec/ids/view-type/anonymous-only, spec/ids/package/as-operators, spec/default-field-rows/per-field

### 1.3 Volatile relations

A volatile relation is a relation kind outside serializable history.
Only the engine declares volatile relations; a user declaration with
the `volatile` suffix is refused.

```adt
relation sys::ctor_stats : CtorStats volatile
  key (relation, ctor)
```

A read of a volatile relation:

- returns the state at some moment during the read, not the state at
  the reading transaction's snapshot;
- may return different rows when repeated in one transaction, and rows
  of two volatile relations read in one query need not describe one
  moment;
- records no read dependency, takes no lock and never causes a
  serialization failure.

An expression is volatile if it reads a volatile relation or calls a
function whose body is volatile. A volatile expression must not occur:

- in a constraint, a row check, a stored view, a default, a generated
  or computed field, an index expression or a delete action;
- in a feed's constructor list ([interop.md](interop.md#52-change-streams));
- in any part of a write: its target, its predicate, the values it
  writes, or a value or condition the write depends on, including a
  `let` binding in a `transact` block whose value reaches a write.

A program that breaks this rule is ill-typed. A registered query or
transaction containing it is refused at registration; ad hoc text
containing it is refused with `Rejected { reason: Invalid { ... } }`.

```adt fragment
type AuditCount = { id: Key audit_counts, rows: I64 }
relation audit_counts : AuditCount key id generate sequence
insert audit_counts from (from h in sys::ctor_stats select { rows: h.live_rows })
-- refused: a volatile source in a write
```

A registered query may read volatile relations and return their rows;
its results carry no isolation guarantee for those rows. A `transact`
block may read a volatile relation and return what it read, provided
no write depends on it.

Some volatile relations are kept in the database file (§5.4, §11.1).
They are written by the engine outside user transactions and are
still volatile: being kept says how long rows last, not how they are
read.

### 1.4 Visibility

Reading any `sys::` relation, `catalog::plans` or the plan of
another principal's query requires the `monitor` privilege (§9.3) or
a `read` grant on the relation.
`monitor` shows counts, timings and plan shapes. A column whose
values reveal keys or field values of a constructor relation also
needs read on that constructor relation's rows and fields
([security.md](security.md#3-grants)). These columns are:

| Relation | Columns |
| --- | --- |
| `sys::field_values` | `value` |
| `sys::conflicts` | `keys` |
| `sys::verify_findings` | `witness` of `Violation` |
| `catalog::plans`, explain results | probe bounds and literal parameter values in `text` and `document` |
| `catalog::audit` | `changes` of `Commit`, which needs read on every row it covers ([security.md](security.md#8-audit)) |

Rows are never silently omitted. A query that reads such a column
without the read it needs is refused with `Rejected { reason: Denied
{..} }`; a query that reads only the other columns is not. For a
registered query running under definer rights, the bundle owner's
privileges decide.

### 1.5 Commit times and safe snapshots

Two `sys::` relations describe the retained history read by `at`
blocks ([derived.md](derived.md#6-reading-the-past)):

```adt
type CommitTime = { position: Position, at: Instant }
relation sys::commit_times : CommitTime volatile key position

type SafeSnapshotMark = { position: Position }
relation sys::safe_snapshots : SafeSnapshotMark volatile key position
```

A row of `commit_times` is one commit within the retention window and
the wall time recorded for it; times never decrease along positions. A
row of `safe_snapshots` is one safe snapshot within the window, named
by the position it follows. `at commit p` reads the greatest row of
`safe_snapshots` at or before `p`, and `at instant t` the greatest at
or before the latest commit whose `at` is at or before `t`. Rows leave
both relations as the window passes them.

Rulings: engine/catalog-relations, ops/expose/relations, ops/expose/volatile/volatile-kind, ops/conflicts/leak/admin-only, project/spec/normative/catalog, spec/ctor-ids/log-order, spec/evolution/new-ids/listed-order, spec/evolution/object-ids/every-object, spec/catalog-key-values/declared

## 2. Catalog relations

### 2.1 Types and constructors

```adt
module catalog

type TypeDef { id: Key types, module: Text, name: Text, params: I32,
               dropped: Option Key migrations }
  = SumType { ordered: Bool }
  | Newtype { over: Text }
relation types : TypeDef key id

type Constructor = { id: Key constructors, sum: Key types, name: Text, position: I32,
                     dropped: Option Key migrations }
relation constructors : Constructor key id

type CtorStage = Declared | Enabled | Deprecated
type CtorState = { relation: Key relations, ctor: Key constructors, stage: CtorStage }
relation ctor_states : CtorState key (relation, ctor)
type ValueState = { ctor: Key constructors, stage: CtorStage }
relation value_states : ValueState key ctor

type Field { id: Key fields, name: Text, field_type: Text, position: I32,
             dropped: Option Key migrations }
  = Header  { sum: Key types }
  | Payload { ctor: Key constructors }
relation fields : Field key id
```

`field_type` is the field's type printed in the notation of
[syntax.md](syntax.md#4-types). `position` is the declaration order,
counted from 1 within the sum for a constructor and within the header
or the constructor for a field; a record is a `SumType` with one
constructor. Every type the engine declares has a row, `Tag` among
them: `Tag` is a `SumType` of one parameter with no constructors of
its own. A constructor's
`dropped` is set by `remove constructor`; its name may then be reused
by a later constructor, which gets a new id.

Write state is per relation variable: `ctor_states` has a row for each
relation variable and each constructor of its type, with the stage of
[evolution.md](evolution.md#43-enable-and-deprecate): `Declared` after
`add constructor` or `disable`, `Enabled` after `enable`, `Deprecated`
after `deprecate`. Constructors created by `add type` start `Enabled`.
The products of split and unnest take their source's stage; the
product of a merge or nest is `Enabled` when every source is enabled
and `Declared` when some source is declared
([evolution.md §4.3](evolution.md#43-enable-and-deprecate)).

Each constructor of a declared sum that a field of any declared type
can hold, at any depth, has one row of `value_states`, prelude sums
included, kept per type, with the same stages and the same
operators naming the type (`enable T.C`, `disable T.C`, `deprecate
T.C`). Its stage gates storing a value of the constructor in any
field, at any depth.

### 2.2 Relation variables, named sets and indexes

```adt
type RelKind = Base | Stored | Catalog | Volatile
type Relation = { id: Key relations, module: Text, name: Text, sum: Key types,
                  kind: RelKind, dropped: Option Key migrations }
relation relations : Relation key id

type NamedSet = { id: Key named_sets, relation: Key relations, name: Text,
                  dropped: Option Key migrations }
relation named_sets : NamedSet key id
type SetMember = { set: Key named_sets, ctor: Key constructors }
relation set_members : SetMember key (set, ctor)

type IndexKind = PrimaryKey | Unique | Plain | Interval | Implied
type Index = { id: Key indexes, relation: Key relations, kind: IndexKind, clause: Text,
               dropped: Option Key migrations }
relation indexes : Index key id
type IndexCtor = { index: Key indexes, ctor: Key constructors }
relation index_ctors : IndexCtor key (index, ctor)
```

`Stored` is a stored view ([derived.md](derived.md#2-stored-views)).
The relations of `catalog` have kind `Catalog` and those of `sys` kind
`Volatile`. An index's `clause` is its canonical index clause, with no
leading or trailing whitespace, comments or line breaks:

- A primary key is `key id` for one field or `key (src, dst)` for several,
  in key-field order, without a `generate` clause.
- Other declared indexes begin with `unique`, `index` or `interval index`,
  followed by one space and their elements in parentheses, in declaration
  order, separated by a comma and one space.
- Field paths have no spaces around `.`. Expression calls have no space
  before `(` and separate arguments with a comma and one space. Binary
  operators have one space on either side; unary `-` has no following
  space, and `not`, `key` and `try` have one. A binary or conditional
  expression used as a unary or binary operand or as the base of a field
  selection is parenthesised. A conditional is `if e then a else b`,
  with one space between its parts. Other redundant parentheses are
  omitted. Names retain their qualification with no spaces around `::`;
  numeric and duration literals retain their spelling, Boolean literals
  are `true` and `false`, and text literals are quoted and escaped.
- An unscoped index has no `in` suffix and ranges over every constructor.
  An explicit scope is resolved to constructor names, expanding a named
  set and ordering its members by the sum's constructor order. One member
  is ` in Google`; several are ` in (Google | Apple)`, with one space
  around each `|`. An explicit scope retains its suffix even when it
  contains every constructor.

For example, `index ( lower( email ) ) in Google` has clause
`index (lower(email)) in Google`. `index_ctors` holds the resolved
constructor set the index ranges over.

An index of kind `Implied` is the implied index of a reference field
([constraints.md §2.1](constraints.md#21-keys-uniques-and-references)):
its `clause` is written as the declaration of an index on the field
would be, `index (owner)` for a header field and `index (user) in
Google` for a field of constructor `Google`, and its `index_ctors` are
the constructors holding the field. It is the index of the reference
held in the one field its clause names. The relation variables of
`catalog` and `sys` have no implied indexes.

### 2.3 Constraints, functions and extensions

```adt
type Constraint = { id: Key constraints, module: Text, name: Text, text: Text,
                    dropped: Option Key migrations }
relation constraints : Constraint key id

type Function { id: Key functions, module: Text, name: Text, signature: Text,
                deterministic: Bool, hash: Bytes, dropped: Option Key migrations }
  = Language  { body: Text }
  | Wasm      { code: Key wasm_modules, export: Text }
  | Host      { symbol: Text, version: Text }
  | Aggregate { body: Text }
relation functions : Function key id

type WasmModule = { hash: Bytes, bytes: Bytes }
relation wasm_modules : WasmModule key hash

type Extension = { id: Key extensions, name: Text, version: Text, hash: Bytes, module: Text }
relation extensions : Extension key id

type Default = { id: Key defaults, relation: Key relations, field: Key fields, text: Text,
                 dropped: Option Key migrations }
relation defaults : Default key id

type Computed = { id: Key computed, relation: Key relations, name: Text, field_type: Text,
                  text: Text, dropped: Option Key migrations }
relation computed : Computed key id
```

A row of `defaults` is one default or generated value of a relation
variable's field ([derived.md](derived.md#4-defaults)); a `default`
clause that fills the field of several constructors has one row per
field, each with the clause's text. A generated key's `text` is its
`generate` clause as written, for example `generate sequence`. A row of
`computed` one computed field
([derived.md](derived.md#5-computed-fields)); `text` is the
expression as written. A stored view is a row of `relations` of kind
`Stored`, defined by a row of `functions`.

A function's `signature` and `body` are as written in its declaration;
an aggregate's `body` is everything after the `=` of its declaration.
A function written in the language is `deterministic` when every
function it calls, directly or through other functions, is
deterministic; a call to a host function not declared `deterministic`
makes it, and every function that calls it, nondeterministic.

Every hash in the catalog is BLAKE3-256. A function's `hash` is the
content hash that is part of its identity
([interop.md](interop.md#95-function-identity)); a Wasm module is keyed
by the hash of its bytes.

### 2.4 Versions, bundles and registered calls

```adt
type Migration = { version: Key migrations, hash: Bytes, text: Text, applied: Instant }
relation migrations : Migration key version

type CtorTarget = InRelation { relation: I64, ctor: I64 }
                | OfType     { ctor: I64 }
type Place = Last | Before { ctor: I64 } | After { ctor: I64 }

type Operator
  = AddModule         { module: Text }
  | DropModule        { module: Text }
  | Install           { module: Text, version: Text, package: Bytes }
  | Uninstall         { module: Text }
  | AddType           { sum: I64, decl: Term }
  | DropType          { sum: I64 }
  | AddRelation       { relation: I64, decl: Term }
  | DropRelation      { relation: I64 }
  | AddSet            { set: I64, relation: I64, name: Text }
  | DropSet           { set: I64 }
  | AddConstructor    { ctor: I64, sum: I64, decl: Term, place: Place }
  | RemoveConstructor { ctor: I64 }
  | Enable            { target: CtorTarget }
  | Disable           { target: CtorTarget }
  | Deprecate         { target: CtorTarget }
  | RenameConstructor { ctor: I64, name: Text }
  | RenameType        { sum: I64, name: Text }
  | RenameRelation    { relation: I64, name: Text }
  | RenameSet         { set: I64, name: Text }
  | RenameField       { field: I64, name: Text }
  | Split             { ctor: I64, decl: Term, routing: Term }
  | Merge             { ctor: I64, decl: Term, routing: Option Term }
  | Nest              { ctor: I64, decl: Term }
  | Unnest            { ctor: I64 }
  | AddField          { field: I64, decl: Term, fill: Term }
  | DropField         { field: I64 }
  | AlterField        { field: I64, action: Term }
  | MoveField         { field: I64, header: Bool, fill: Option Term }
  | AddUnique         { index: I64, relation: I64, fields: Term }
  | DropUnique        { index: I64 }
  | Rekey             { relation: I64, key: I64, fields: Term }
  | AddIndex          { index: I64, relation: I64, interval: Bool, fields: Term }
  | DropIndex         { index: I64 }
  | AddDefault        { default: I64, relation: I64, field: I64,
                        value: Term }
  | DropDefault       { default: I64 }
  | DefineFn          { function: I64, decl: Term }
  | RedefineFn        { function: I64, decl: Term }
  | DropFn            { function: I64 }
  | DefineAggregate   { function: I64, decl: Term }
  | DropAggregate     { function: I64 }
  | AddConstraint     { constraint: I64, body: Term }
  | DropConstraint    { constraint: I64 }
  | AddCheck          { constraint: I64, body: Term }
  | DropCheck         { constraint: I64 }
  | Store             { view: I64, function: I64, decl: Term }
  | Unstore           { view: I64 }
  | AddComputed       { computed: I64, relation: I64, name: Text,
                        decl: Term }
  | RedefineComputed  { computed: I64, decl: Term }
  | DropComputed      { computed: I64 }
  | Grant             { privilege: Text, target: Option Term, grantee: Text, grant_option: Bool }
  | Revoke            { privilege: Text, target: Option Term, grantee: Text }
  | AdoptTzdb         { release: Text, data: Bytes }
  | Script            { write: Term }
  | Revert            { migration: I64 }
type Term = new Text
type MigrationOp = { migration: Key migrations, ordinal: I32, op: Operator }
relation migration_ops : MigrationOp key (migration, ordinal)

type ArmTarget = OfFn         { function: I64 }
               | OfConstraint { constraint: I64 }
               | OfComputed   { computed: I64 }
type ClauseBody = Joins       { set: I64 }
                | Member      { ctor: I64 }
                | Arm         { target: ArmTarget, arm: Term }
                | Replacement { index: I64, kind: IndexKind, fields: Term }
                | Carry       { field: I64 }
                | Fill        { field: I64, fill: Term }
                | Privilege   { privilege: Text, target: Option Term }
type OpClause = { migration: Key migrations, ordinal: I32, clause: I32, body: ClauseBody }
relation migration_clauses : OpClause key (migration, ordinal, clause)

type Priority ordered = AdHoc | Background | Regular
type BundleState = Active | Superseded { at: Instant } | Retired { at: Instant }
type Bundle = { id: Key bundles, name: Text, family: Text, owner: Text,
                pins: Key migrations, hash: Bytes, edition: Text, catalog: Option I64,
                hides_new: Bool, lease: Option Duration, renewed: Instant,
                priority: Priority, memory_share: Option F64, workers: Option I32,
                state: BundleState }
relation bundles : Bundle key id

type Registered { id: Key registered, bundle: Key bundles, name: Text, ordinal: I32,
                  signature: Text, text: Text, limit: Option Duration }
  = Query {}
  | Txn {}
  | ImportedFn {}
relation registered : Registered key id

type Feed = { id: Key feeds, bundle: Key bundles, name: Text, ordinal: I32,
              relation: Key relations, set: Text }
relation feeds : Feed key id
type FeedMember = { feed: Key feeds, relation: Key relations, ctor: Key constructors }
relation feed_members : FeedMember key (feed, relation, ctor)
```

`migrations` is the operator log of
[evolution.md](evolution.md#1-the-operator-log), one row per migration;
`hash` is the version's hash, the BLAKE3-256 hash of the log prefix
up to and including this migration, `text` the migration as
written, and `applied` the wall-clock time at which it was applied,
read in the transaction that applied it.

`migration_ops` holds each migration's resolved operators, one row per
operator, `ordinal` its position in the migration from 1. Each
`Operator` constructor is one operator of
[evolution.md §4](evolution.md#4-operators), and names every object it
touches by its catalog id: the object it creates takes the id of
§1.2. `Operator`, `Place`, `CtorTarget`, `ArmTarget` and `ClauseBody`
are value-level sums, which hold no references
([model.md §8.2](model.md#82-where-references-may-occur)), so each of
their fields that names an object holds the object's catalog id as an
`I64`. The engine keeps every such id naming a row of its relation, and
a reader joins on it:

```adt
from o in catalog::migration_ops, c in catalog::constructors
where match o.op
  | AddConstructor a -> catalog::constructors.key(a.ctor) == key c
  | _                -> false
select { migration: o.migration, ctor: c.name }
```

`Bytes` operands of `Install` and `AdoptTzdb` hold the BLAKE3-256
hash of the package or release data, which the log stores beside it.
`header` is set when a `move field` moves the field to the header. An
operand written in the language (a declaration, a fill, a routing
`match`, a key or index field list, a body, a write) is a `Term`: the
canonical token form of the resolved operand
([syntax.md §2.9](syntax.md#29-canonical-token-form)), with every Wasm
module it names written as the module's hash. The `decl` of `Store` is
the stored view's declaration, `store f` with its clauses, as a `Term`.
The `body` of `AddConstraint` and `AddCheck` is the whole declaration
after the verb, so a check's `body` holds its `on` target. A `fields`
Term is the field list without its parentheses: `email, name` for
`add unique (email, name) in users`.

The clauses an operator takes any number of are rows of
`migration_clauses`, `clause` their position from 1: `Joins` for a
`joins` set, `Member` for each constructor a merge or nest replaces, a
named set lists, a moved field goes to, an `add default` or `drop
default` fills or the written `in r.C` scope of an index or unique
names, `Arm` for an `arm` clause, `Replacement`
for the `unique` and
`index` clauses of a split, `Carry` for a field after `carry`,
`Fill` for a revert's `fill`, and `Privilege` for each privilege of a
grant or revoke after its first. An index or unique added without a
scope has no `Member` clauses and ranges over every constructor of its
relation, constructors added after it included.

A `Grant` or `Revoke` holds the list of privileges its statement
names, in written order, each with its own target: `privilege` and
`target` are the first, and each `Privilege` clause one more. A
privilege's `privilege` is its name and its `target` the `Term` of
what it is granted on. A `Grant` or `Revoke` whose `target` is `None`
and that has no `Privilege` clauses holds its whole privilege list in
`privilege`, in canonical token form, and is read as that list; no
other operator privilege has `target` `None`. An `AddDefault` names every constructor its default
applies to: `field` is the header field it fills, or the field of the
first constructor it fills in declared order, and a `Member` clause
names each constructor whose field it fills, in declared order, none
for a header field. A `DropDefault` drops every default the matching
`drop default` removes: `default` is the first of them, and a
`Member` clause names each constructor whose default it drops, in
declared order. The version hash
([evolution.md §2.1](evolution.md#21-position-and-hash)) encodes these
rows.
The log holds only migrations; bundles, their registration, leases,
supersession and retirement are deployment state in
`catalog::bundles`, not log entries. A bundle's name is
`FAMILY-VERSION`, `VERSION` being the digits and dots after the last
hyphen, and `family` is the part before it (`web` for `web-2.4`).
`owner` is the registering role, and the owner of a family is the
owner of its first registered bundle; `catalog` is the catalog version the
bundle pins, if it reads system relations; `hides_new` is set by the
header clause `hide new constructors`
([evolution.md](evolution.md#8-readers-first)); `renewed` is the last lease renewal
([applications.md](applications.md#42-leases)). A bundle is marked
`Superseded` when an embedded open registers its successor, which
commits durably before the open's migrations, or by the command
`supersede bundle` ([applications.md](applications.md#44-supersession)).
A `Superseded` bundle is not a live reader. Registering it again with
identical content runs checks 3 to 5 of
[applications.md §4.1](applications.md#41-registration) and may be
refused with `BundleStale` or `BundleExpired`; a `Retired` one is
retired for good.

`text` is the declaration as the bundle file writes it, followed by
each bundle function the file declares, and for an imported function
its `import fn` line; `signature` is its parameters and result type.
`ordinal` is the call's position in declaration order in the bundle
file, from 1, so code generation computes it from the file
([wire.md](wire.md#51-call)); queries, transactions, imported
functions and feeds of one bundle share one ordinal sequence. `limit`
is the declared limit of §8.3. A row of `feeds` is a feed the bundle
declares, with `set` its constructor set as written and its
constructor relations at the pinned version in `feed_members`
([interop.md](interop.md#52-change-streams)).

### 2.5 Plans

```adt
type PlanState = Current | Pinned | Reverted | Superseded
type Plan = { id: Key plans, query: Key registered, release: Text, created: Instant,
              state: PlanState, memory_grant: I64, text: Text, document: Text }
relation plans : Plan key id
```

Plans are defined in §7.

### 2.6 Other catalog relations

| Relation | Defined in |
| --- | --- |
| `catalog::settings` | §10.2 |
| `catalog::tz_releases`, `catalog::zones` | §3.1 |
| `catalog::roles`, `catalog::credentials`, `catalog::grants`, `catalog::peer_mappings` | below; [security.md](security.md#2-authentication) |
| `catalog::audit` | [security.md](security.md#82-the-audit-relation) |

Roles, their credentials, grants and peer mappings:

```adt
type Role = { id: Key roles, name: Text, created: Instant }
relation roles : Role
  key id
  unique (name)

type Credential = { id: Key credentials, role: Key roles, salt: Bytes, iterations: I32,
                    stored_key: Bytes, server_key: Bytes }
relation credentials : Credential
  key id
  unique (role)

type GrantKind = Log | Deployment
type RoleGrant = { id: Key grants, kind: GrantKind, privilege: Text, target: Option Text,
                   grantee: Text, grantor: Text, grant_option: Bool, dormant: Bool }
relation grants : RoleGrant key id

type PeerMapping = { listener: Text, ordinal: I32, pattern: Text, role: Text }
relation peer_mappings : PeerMapping key (listener, ordinal)
```

A row of `credentials` is a SCRAM-SHA-256 verifier
([security.md](security.md#21-roles)). A row of `grants` is one
privilege of one grant statement: `kind` is `Log` for a grant operator
of the log and `Deployment` for a grant given by ad hoc statement
([security.md](security.md#32-grant-forms)); `privilege` is the
privilege's name; `target` is what it is granted on (a constructor
set with its fields, a bundle, a query, a family or a function),
`None` for a capability. A `Log` row's `target` names every object by
its current name and follows the grant through renames and the
carrying of [security.md §3.6](security.md#36-grants-across-schema-changes):
the relation as `module::name`, followed, unless the grant is on the
whole relation, by `.C`, `.(C | D)` or `.S`, constructors in id
order; a `read` or `update` with a field list then has the
listed fields, and one without a list the fields it covers after
`frozen`, in field id order: `auth::identities.EmailPass (user,
EmailPass.email)`, `auth::identities.EmailPass frozen (id, user,
EmailPass.email, EmailPass.hash)`; a `retag` is `a -> b`, followed by
`except (C -> D, …)` when it excepts pairs, in id order of the first
constructor, then of the second; a `call` is the function
as `module::name`. A header field, or a field of a type with one
constructor, is named alone, and any other field as `C.f`. A call
grant on a family has the target `FAMILY::q at v`, then the signature
it records, its parameters and result type as a query declaration
writes them, with each schema type and relation as `module::name` as
it stands at log version `v`, the version recorded when it was
granted: `web::login_key at 41 (u: Key auth::users) -> Text`. No two rows have the same grantee, privilege,
target and grantor. `grantee` and `grantor` are role
names. `dormant` marks a log grant whose grantee did not exist when
the log was replayed; a dormant grant allows nothing. Creating the
role does not clear the mark. A later migration that grants the same
privilege on the same target to the role clears it, keeping the row's
`id`, and the row then has that grant's `grant_option`; a grant
without a field list whose fields differ from the dormant row's is
another privilege and has a row of its own. `revoke` deletes the
row. A row of `peer_mappings` is one rule of a listener's
peer mapping ([security.md](security.md#22-sources)), tried in
`ordinal` order.

Rulings: engine/catalog-relations, project/spec/normative/catalog, spec/evolution/product-state/source-state, spec/evolution/operator-type/catalog-sum, spec/evolution/version-hash/value-encoding, spec/evolution/term-encoding/canonical-tokens, spec/evolution/object-ids/every-object, spec/catalog-refs/key-value, spec/default-field-rows/per-field

## 3. Zone rules and collations

The rules for time-zone conversion and for collation are given in
[model.md](model.md#36-time) and [model.md](model.md#35-text). This section
states only where their data lives.

### 3.1 Zone rules

Time-zone rules are catalog data:

```adt
type TzRelease = { id: Key tz_releases, release: Text, adopted: Key migrations }
relation tz_releases : TzRelease
  key id
  unique (release)
type Zone = { release: Key tz_releases, name: Text, rules: Bytes }
relation zones : Zone key (release, name)
```

The release most recently adopted is in force. Every conversion
between `Instant` and civil or zoned time uses it, for every bundle
whatever version it pins. A stored `ZonedDateTime` keeps its civil
time and zone; its `Instant` is computed with the release in force.

Adopting a release is a schema operator. Its operator value holds the
BLAKE3-256 hash of the release's data, so the version hash covers it,
and the log and dumps carry the data itself. From
version 17:

```adt fragment
-- 0018
module auth
adopt tzdb "2026b" from "tzdata-2026b.tzif"
```

The operator must report every stored `ZonedDateTime` whose `Instant`
changes under the new release. Indexes, stored views and constraints
whose expressions convert through zone rules are rebuilt or rechecked
as part of the operator, as for a redefined function
([interop.md](interop.md#95-function-identity)). The inverse operator
restores the previous release.

### 3.2 Collations

`Text` orders by code point. A collation is a function from `Text` to
an ordered sort key, named by a BCP 47 locale and the CLDR release
whose collation data it follows, such as `"de-u-ks-level2@cldr-49"`.
The root collation is `und`. A well-formed locale with no tailoring of
its own in that release collates as CLDR's fallback gives, in root
order unless a parent locale is tailored. The engine never changes the
tables behind a collation name. An engine release must support every
collation name any earlier release supported; it may add names.

```adt
type Person = { id: Key people, name: Text }
relation people : Person
  key id generate sequence
  index (collate(name, "de@cldr-49"))
```

The volatile relation `sys::collations` lists, one row per name, the
root collation and every tailored locale of each CLDR release the
binary supports:

```adt
type Collation = { id: Key collations, name: Text }
relation sys::collations : Collation volatile
  key id
  unique (name)
```

A well-formed locale the relation does not list is still a name, by
fallback. Moving an index,
constraint or function from one collation name to another is a schema
change (dropping and adding the index, or redefining the function),
which rebuilds what depends on it.

Rulings: types/tzdb/catalog, types/collation/sortkey-fn, types/collation/version/versioned-name, spec/collation-names/cldr, spec/collation-fallback/root

## 4. Error sums

### 4.1 The error sum

Every failure the engine reports to a caller, through the server
protocol or the in-process API, is a value of the prelude sum
`Error`. Its constructors stand for what the caller does; the reason
inside each says why.

```adt
type Error = Retry    { reason: RetryReason }
           | Rejected { reason: RejectReason }
           | Fatal    { reason: FatalReason }
```

| Constructor | Meaning for the caller |
| --- | --- |
| `Retry` | Nothing was applied. Running the same call or transaction again may succeed. |
| `Rejected` | Nothing was applied. The same request fails again until something outside it changes: report it, fix it, or redeploy. |
| `Fatal` | The connection or the part of the database the request needs cannot serve it. An operator or the driver must act. |

Values carry no references, so reasons name catalog objects by
qualified name in `Text` fields (`identities.Google`,
`web-2.4::login_key`).

An embedded open that fails returns an `OpenError` of the in-process
API instead ([embedding.md](embedding.md#21-open)), among them
`HostFnMissing` and `HostFnMismatch` for a host function that is not
supplied or does not match its declaration; a migration or
registration refused inside the open carries its `Error`.

### 4.2 Reasons

```adt
type TxnId = new I64
type Report = new Bytes
type Position = { epoch: I64, lsn: I64 }

type RetryReason = Serialization      { txn: TxnId, on: Text }
                 | SafePointTimeout   { waited: Duration }
                 | Overloaded         { retry_after: Duration }
                 | StreamIdle         { limit: Duration }
                 | FileChanged        { path: Text }
                 | Fenced
                 | ReplayConflict     { waited: Duration }

type RejectReason = Violated             { constraint: Text, report: Report, redacted: I64 }
                  | KeyViolation         { relation: Text, key: Text }
                  | KeyChanged           { relation: Text }
                  | NotOpen              { ctor: Text, stage: Text }
                  | UserError            { message: Text }
                  | Faulted              { fault: Fault }
                  | Denied               { detail: Text }
                  | Invalid              { detail: Text }
                  | SchemaRefused        { operator: Text, detail: Text }
                  | Untranslatable       { query: Text, detail: Text }
                  | BadInput             { source: Text, line: I64, detail: Text }
                  | SchemaMismatch       { detail: Text }
                  | Cancelled
                  | Terminated
                  | DeadlineExceeded
                  | StatementLimit       { limit: Limit }
                  | TransactionExpired   { limit: Duration, idle: Bool }
                  | TransactionBusy
                  | MixedVersions
                  | UnknownQuery
                  | UnknownTransaction
                  | UnknownBundle        { bundle: Text }
                  | VersionMismatch      { bundle: Text, version: I64 }
                  | CatalogMismatch      { bundle: Text, catalog: I64 }
                  | Retired              { bundle: Text }
                  | BundleExpired        { bundle: Text, query: Text }
                  | BundleStale          { bundle: Text, ctor: Text }
                  | BundleSuperseded     { bundle: Text }
                  | DowngradeRefused     { cause: Text }
                  | TooOld               { oldest: Position }
                  | HistoryUnavailable   { oldest: Position }
                  | PositionNotOnHistory { position: Position }
                  | WrongVersion         { version: I64 }
                  | ReadOnlyReplica      { primary: Text }
                  | RequestExpired
                  | RequestReused
                  | AuthFailed
                  | UnsupportedProtocol  { min: Text, max: Text }
                  | UnknownDatabase      { database: Text }
                  | EditionLocked
                  | UnknownEdition       { edition: Text }
                  | MigrationInProgress
                  | DumpDamaged          { offset: I64 }
                  | Internal             { what: Text }
                  | RowTooLarge          { relation: Text, ctor: Text, size: I64, limit: I64 }
                  | KeyTooLarge          { relation: Text, ctor: Text, index: Text, size: I64, limit: I64 }
                  | Closed

type Limit = Time   { after: Duration }
           | Memory { bytes: I64 }
           | Rounds { rounds: I64 }

type FatalReason = Malformed       { detail: Text }
                 | Quarantined     { what: Text }
                 | MissingFunction { function: Text }
                 | Storage         { detail: Text }
```

`Fault` is the prelude sum of faults ([prelude.md](prelude.md)). A
`TxnId` is a random positive 63-bit integer. `Position` values fit the
wire's 32-bit epoch and 64-bit log position.

| Reason | When | Defined in |
| --- | --- | --- |
| `Serialization` | the transaction lost a serialization conflict, or conflicted with, waited on, or collided on a key with a row committed after its snapshot; `txn` is the caller's aborted transaction, which names the conflict's row in `sys::conflicts`, and `on` the constructor relation of the caller's own read or write on the conflicting edge | [transactions.md](transactions.md#61-serialization-failures), §6.4 |
| `SafePointTimeout` | a replica read given a position waited the `ReplicaWait` setting without the replica reaching a safe point at or after it | [replication.md](replication.md#12-reads-on-a-replica) |
| `Overloaded` | a connection, stream or admission queue bound was reached | [wire.md](wire.md#8-admission-and-overload), §8.1 |
| `StreamIdle` | a result stream was given no credit for longer than the limit; never sent after the call's commit | [wire.md](wire.md#55-credit) |
| `FileChanged` | a file read twice in one transaction changed between the reads | [interop.md](interop.md#33-file-integrity) |
| `Fenced` | a commit reached a primary fenced by a promotion | [replication.md](replication.md#2-failover) |
| `ReplayConflict` | a replica read was cancelled because applying the log would remove a row version its snapshot sees, after the replay waited `waited`, the `ReplicaVacuumWait` setting | [replication.md](replication.md#12-reads-on-a-replica) |
| `Violated` | the change set violated a constraint or a unique, including a duplicate under a unique; `constraint` is the violated constraint whose qualified name sorts first, `report` its result rows (below) | [constraints.md](constraints.md#7-checking-and-violation-reports) |
| `KeyViolation` | two different rows with one value of the relation's key are inserted in one change set, an appended batch included, or an insert gives a key that a pre-state row the change set does not delete holds, even with an equal value; reported at the statement boundary; `key` is the key in the textual syntax. A duplicate under any unique is `Violated` with that unique's constraint | [writes.md](writes.md#14-combining-writes), [wire.md](wire.md#73-append) |
| `KeyChanged` | an update gave a row another key | [writes.md](writes.md#3-update) |
| `NotOpen` | an insert or retag into a constructor relation in a stage that refuses writes, or a write storing in a field a value of a value-level constructor in such a stage; `ctor` names the constructor qualified by its relation (`identities.Apple`) or by its sum (`Provider.Okta`) | [evolution.md](evolution.md#43-enable-and-deprecate) |
| `UserError` | a write evaluated `fail e`; `message` is the value of `e` | [writes.md](writes.md#62-meaning) |
| `Faulted` | the statement aborted on a fault | [queries.md](queries.md#15-faults) |
| `Denied` | the principal lacks a privilege, or a query reads a column the role may not see | [security.md](security.md#3-grants), §1.4 |
| `Invalid` | ad hoc text failed to parse or type-check, a command does not apply, or an argument is outside its parameter's domain | [typing.md](typing.md) |
| `SchemaRefused` | a schema operator was refused | [evolution.md](evolution.md#32-checks-at-apply) |
| `Untranslatable` | a versioned document (interop §1.2) is read through a lossy step of translation | [interop.md](interop.md#12-versioned-documents), [evolution.md](evolution.md#71-serving-a-pinned-version) |
| `BadInput` | a row of an external source did not convert or route, with no rejects relation | [interop.md](interop.md#45-rejects) |
| `SchemaMismatch` | an appended Arrow batch does not have the constructor's schema | [embedding.md](embedding.md#9-bulk-append) |
| `Cancelled` | the call was cancelled | §8.4 |
| `Terminated` | the call's session was terminated | §8.4 |
| `DeadlineExceeded` | the call's deadline passed | §8.3 |
| `StatementLimit` | the statement ran past a limit, which `limit` names: `Time` its statement limit or declared limit, after `after`, `Memory` a memory grant of `bytes` it could not stay within by spilling, or an admission byte bound of `bytes` its arguments exceed (§8.1), `Rounds` the fixpoint round limit ([semantics.md §5.10](semantics.md#510-fixpoints)) | §8.3 |
| `TransactionExpired` | an interactive transaction passed its transaction limit, or its idle limit when `idle`; rerunning it unchanged hits the same limit | [transactions.md](transactions.md#54-time-limit), §8.3 |
| `TransactionBusy` | the interactive transaction is in use from another connection | [wire.md](wire.md#62-interactive-transactions) |
| `MixedVersions` | a call would run at another schema or catalog version than its transaction's earlier statements, as when a transaction that has made a registered call sends ad hoc text, or one that has run ad hoc text makes a registered call | [transactions.md](transactions.md#2-statements), [applications.md](applications.md#7-the-ad-hoc-role) |
| `UnknownQuery`, `UnknownTransaction`, `UnknownBundle`, `VersionMismatch` | the request names a call, transaction or bundle the binding does not have | [wire.md](wire.md#34-binding-and-ready) |
| `CatalogMismatch` | a Hello binding names a catalog version other than the one the bundle pins; `catalog` is the pinned one | [wire.md](wire.md#32-hello) |
| `Retired` | the bundle is retired; bound connections are refused too | [applications.md](applications.md#43-retirement) |
| `BundleExpired` | a bind, call or lease renewal of an expired bundle, on the primary or a replica; or an expired or superseded bundle registered again fails the checks of a re-registration | [evolution.md](evolution.md#73-expired-bundles), [applications.md](applications.md#42-leases) |
| `BundleStale` | a bundle registering at a version before an enabled constructor it would read | [applications.md](applications.md#41-registration) |
| `DowngradeRefused` | an embedded open with an older bundle whose reads include a constructor enabled since, or whose host function was redefined since; `cause` names it, and nothing is changed | [embedding.md](embedding.md) |
| `BundleSuperseded` | a binding names a bundle an embedded open or `supersede bundle` superseded and that has not been registered again | [applications.md](applications.md#8-embedded-databases) |
| `TooOld` | a feed position is older than retention; `oldest` is the oldest position kept | [wire.md](wire.md#9-feeds) |
| `HistoryUnavailable` | a time-travel read names a position outside the retention window or after the reader's snapshot, or a feed position is before the feed's registration | [derived.md](derived.md#6-reading-the-past), [wire.md](wire.md#9-feeds) |
| `PositionNotOnHistory` | a position names an epoch or log position not on this database's history | [transactions.md](transactions.md#8-commit-positions), [replication.md](replication.md#2-failover) |
| `WrongVersion` | a time-travel read at a pinned version names a position whose relevant types differ from those at the pinned version | [derived.md](derived.md#6-reading-the-past) |
| `ReadOnlyReplica` | a write call reached a replica that does not forward writes, or a load into several relations reached an embedded replica; `primary` names the primary | [replication.md](replication.md#13-writes-at-a-replica) |
| `RequestExpired` | a request id with no record, and not running, is strictly below the client's low-water mark (§6.6); the request is not run | [wire.md](wire.md#65-idempotent-writes) |
| `RequestReused` | a request id already recorded is sent with a different call or arguments | [wire.md](wire.md#65-idempotent-writes) |
| `AuthFailed` | authentication failed | [wire.md](wire.md#33-authentication) |
| `UnsupportedProtocol` | client and server share no protocol version; `min` and `max` are the server's range | [wire.md](wire.md#131-negotiation) |
| `UnknownDatabase` | a Hello names a database the process does not serve | [wire.md](wire.md#32-hello) |
| `EditionLocked` | a Hello asks for an edition on a bound connection, whose edition is its bundle's | [wire.md](wire.md#32-hello) |
| `UnknownEdition` | a Hello asks for an edition the server does not implement | [wire.md](wire.md#32-hello) |
| `MigrationInProgress` | a migration set was presented while a staged migration is pending | [evolution.md](evolution.md#23-applying-a-migration-set) |
| `DumpDamaged` | a restore could not authenticate or decode the dump; `offset` is the byte offset of the first part that fails | [interop.md](interop.md#61-a-logical-dump) |
| `Internal` | the statement broke a guarantee the engine's checker makes about a program, a bug in the engine; the statement fails and none of its effects are applied, the transaction stays open, and the engine keeps serving; `what` is implementation-defined | [transactions.md](transactions.md#52-meaning) |
| `RowTooLarge` | a row of the statement's change set has a stored form larger than the engine's row limit; `relation` is its relation variable qualified by module, `ctor` its constructor, `size` and `limit` in bytes | [writes.md](writes.md#16-rows-too-large) |
| `KeyTooLarge` | a new row's encoded index key payload exceeds its database-derived limit; `relation` is module-qualified, `ctor` is the row's constructor, `index` is the canonical index clause of [§2.2](#22-relation-variables-named-sets-and-indexes), and `size` and `limit` are payload bytes | [writes.md](writes.md#17-index-keys-too-large) |
| `Closed` | an in-process call, interactive transaction or load reached a database after its `close`; it has no effect | [embedding.md](embedding.md#43-errors) |
| `Malformed` | a message violates the server protocol | [wire.md](wire.md#113-malformed-messages) |
| `Quarantined` | the call reads a quarantined constructor relation, or writes a constructor relation a quarantined index covers; `what` names it | §11.2 |
| `MissingFunction` | the call needs a host function the program did not supply | [embedding.md](embedding.md#10-host-functions) |
| `Storage` | an I/O failure or full storage | §11 |

`report` in `Violated` is a ResultType descriptor of the constraint's
result type followed by a Rows body ([wire.md](wire.md#52-replies)).
It holds the violation rows the caller may read; the others are left
out and counted in `redacted` ([security.md](security.md#6-leaks)).
For a registered call the bundle owner's grants decide, and for an ad
hoc call the caller's own.

The server never reruns a transaction that ended with
`TransactionExpired`, `StatementLimit` or `DeadlineExceeded`.

### 4.3 Matching errors

`Error` and its reason sums are closed sums, matched exhaustively like
any sum. A client that only decides what to do matches the three
constructors:

```adt
-- given e: Error
fn rerun () -> Text = "rerun"
fn report (r: RejectReason) -> Text = "report"
fn alert (r: FatalReason) -> Text = "alert"
match e
  | Retry _    -> rerun()
  | Rejected r -> report(r.reason)
  | Fatal f    -> alert(f.reason)
```

Code generation gives each host language a type for `Error` as for
any sum, one per bundle, from the bundle's edition
([embedding.md](embedding.md#43-errors)); the server protocol encodes
errors by constructor id like any sum value
([wire.md](wire.md#4-value-encoding)).

### 4.4 Editions

`Error`, its reason sums, `Notice` (§4.5) and the other prelude sums
are versioned with the edition of the spec; catalog and `sys` sums
evolve through the catalog log instead
([evolution.md](evolution.md#10-engine-releases)). A bundle names its edition
([applications.md](applications.md#3-bundles)), and every error and
notice sent to a call or connection of that bundle is a value of these
sums as that edition declares them. A connection's `Ready` names the
edition it is served ([wire.md](wire.md#34-binding-and-ready)): the
bound bundle's, or for an ad hoc connection the edition its `Hello`
requested, else the newest the server implements. A `Hello` that asks
for an edition on a bound connection is refused with `EditionLocked`,
and one that asks for an edition the server does not implement with
`UnknownEdition`. Every error and
notice value is length-prefixed on the wire, so a client can skip a
reason it does not know.

An edition that adds a constructor to one of these sums must state,
for each added constructor, the constructor of the previous edition
that stands for it. The engine must send a bundle the image of the
value under these mappings, applied edition by edition down to the
bundle's edition. A new constructor of `Error` itself is added only
for a new caller action; a new cause is a new reason.

### 4.5 Notices

The engine tells a bound client of events that concern it by values of
the prelude sum `Notice`, delivered as pushes
([wire.md](wire.md#10-pushes)) or through the in-process notice channel
([embedding.md](embedding.md#45-notices)):

```adt
type Notice = LeaseExpiring      { bundle: Text, at: Instant }
            | ShuttingDown       { at: Instant }
            | ConstructorEnabled { relation: Text, ctor: Text }
            | BundleRetired      { bundle: Text }
            | BlockingMigration  { bundle: Text, version: I64 }
            | UpgradeRequired    { bundle: Text }
```

| Notice | Meaning |
| --- | --- |
| `LeaseExpiring` | the bundle's lease ends at `at` unless renewed ([applications.md](applications.md#42-leases)) |
| `ShuttingDown` | the server accepts calls until `at` and opens no new result stream after it |
| `ConstructorEnabled` | a constructor of a relation the bundle reads was enabled, or a constructor of a value-level sum reached by a field it reads was enabled by `enable T.C`, with `relation` then the sum's qualified type name ([evolution.md](evolution.md#43-enable-and-deprecate)) |
| `BundleRetired` | the bundle has been retired; every later call on the connection is refused with `Retired` |
| `BlockingMigration` | the bundle blocked a migration that would produce schema version `version`, or a revoke issued at schema version `version`, and the migration or revoke was refused; nothing waits, and it succeeds when issued again after the bundle is retired, superseded or expired, which needs every client of it to stop calling (see `sys::sessions`). Sent to the bundle's sessions for the `BlockingNoticePeriod` setting after the refusal |
| `UpgradeRequired` | an operator asks the bundle's clients to upgrade to a newer bundle of its family |

### 4.6 Constructor ids of engine-defined sums

Ids below 1024 are fixed by the spec, so code generation and every
engine agree without reading a log; the operator log assigns ids from
1024 (§1.2). The prelude's ids are fixed per edition, and the ids of
the `catalog` and `sys` modules per catalog version. A type takes one
id and each of its constructors one more; a record or newtype has one
constructor. A constructor that a later edition or catalog version
adds takes the next id not used in any earlier one, so no id ever
changes meaning. A constructor of `Tag T` has the id of the
constructor of `T` it names. A stored value-level tag is the
constructor id.

Edition 2026-10, catalog version 1:

Prelude:

| Id | Type | Constructors |
| --- | --- | --- |
| 0 | `Bool` | 1 `False`, 2 `True` |
| 3 | `Option` | 4 `None`, 5 `Some` |
| 6 | `Result` | 7 `Ok`, 8 `Err` |
| 9 | `Tag` | none of its own: see above |
| 10 | `Rounding` | 11 `HalfEven`, 12 `HalfUp`, 13 `HalfDown`, 14 `Up`, 15 `Down`, 16 `Ceiling`, 17 `Floor` |
| 18 | `Overflow` | 19 `Clamp`, 20 `Reject` |
| 21 | `Fault` | 22 `DivisionByZero`, 23 `ArithmeticOverflow`, 24 `OutOfRange`, 25 `NotFinite`, 26 `NoSuchDate`, 27 `UnknownZone`, 28 `InvalidText`, 29 `InvalidSpan`, 30 `NegativeCount`, 31 `NoSuchRow`, 32 `FunctionFailed` |
| 33 | `Uuid` | 34 (one constructor) |
| 35 | `Span` | 36 (one constructor) |
| 37 | `Principal`, the default only | 38 `EndUser`, 39 `Service`, 40 `Anonymous` |
| 41 | `Origin` | 42 `Registered`, 43 `Function`, 44 `AdHoc` |
| 45 | `Outcome` | 46 `Succeeded`, 47 `Failed` |
| 48 | `Error` | 49 `Retry`, 50 `Rejected`, 51 `Fatal` |
| 52 | `TxnId` | 53 (one constructor) |
| 54 | `Report` | 55 (one constructor) |
| 56 | `Position` | 57 (one constructor) |
| 58 | `RetryReason` | 59 `Serialization`, 60 `SafePointTimeout`, 61 `Overloaded`, 62 `StreamIdle`, 63 `FileChanged`, 64 `Fenced`, 341 `ReplayConflict` |
| 65 | `RejectReason` | 66 `Violated`, 67 `KeyViolation`, 68 `KeyChanged`, 69 `NotOpen`, 70 `UserError`, 71 `Faulted`, 72 `Denied`, 73 `Invalid`, 74 `SchemaRefused`, 75 `Untranslatable`, 76 `BadInput`, 77 `SchemaMismatch`, 78 `Cancelled`, 79 `Terminated`, 80 `DeadlineExceeded`, 81 `StatementLimit`, 82 `TransactionExpired`, 83 `TransactionBusy`, 84 `UnknownQuery`, 85 `UnknownTransaction`, 86 `UnknownBundle`, 87 `VersionMismatch`, 88 `Retired`, 89 `BundleExpired`, 90 `BundleStale`, 91 `BundleSuperseded`, 92 `DowngradeRefused`, 93 `TooOld`, 94 `HistoryUnavailable`, 95 `PositionNotOnHistory`, 96 `WrongVersion`, 97 `ReadOnlyReplica`, 98 `RequestExpired`, 99 `RequestReused`, 100 `AuthFailed`, 101 `UnsupportedProtocol`, 102 `UnknownDatabase`, 342 `CatalogMismatch`, 343 `EditionLocked`, 344 `UnknownEdition`, 345 `MixedVersions`, 372 `MigrationInProgress`, 460 `DumpDamaged`, 465 `Internal`, 467 `RowTooLarge`, 469 `Closed`, 471 `KeyTooLarge` |
| 103 | `FatalReason` | 104 `Malformed`, 105 `Quarantined`, 106 `MissingFunction`, 107 `Storage` |
| 108 | `Notice` | 109 `LeaseExpiring`, 110 `ShuttingDown`, 111 `ConstructorEnabled`, 112 `BundleRetired`, 113 `BlockingMigration`, 114 `UpgradeRequired` |
| 115 | `RejectedRow` | 116 (one constructor) |
| 461 | `Limit` | 462 `Time`, 463 `Memory`, 464 `Rounds` |

Module `catalog`, types:

| Id | Type | Constructors |
| --- | --- | --- |
| 117 | `TypeDef` | 118 `SumType`, 119 `Newtype` |
| 120 | `Constructor` | 121 (one constructor) |
| 122 | `CtorStage` | 123 `Declared`, 124 `Enabled`, 125 `Deprecated` |
| 126 | `CtorState` | 127 (one constructor) |
| 128 | `Field` | 129 `Header`, 130 `Payload` |
| 131 | `RelKind` | 132 `Base`, 133 `Stored`, 134 `Catalog`, 135 `Volatile` |
| 136 | `Relation` | 137 (one constructor) |
| 138 | `NamedSet` | 139 (one constructor) |
| 140 | `SetMember` | 141 (one constructor) |
| 142 | `IndexKind` | 143 `PrimaryKey`, 144 `Unique`, 145 `Plain`, 146 `Interval`, 466 `Implied` |
| 147 | `Index` | 148 (one constructor) |
| 149 | `IndexCtor` | 150 (one constructor) |
| 151 | `Constraint` | 152 (one constructor) |
| 153 | `Function` | 154 `Language`, 155 `Wasm`, 156 `Host`, 157 `Aggregate` |
| 158 | `WasmModule` | 159 (one constructor) |
| 160 | `Extension` | 161 (one constructor) |
| 162 | `Migration` | 163 (one constructor) |
| 164 | `Priority` | 165 `AdHoc`, 166 `Background`, 167 `Regular` |
| 168 | `BundleState` | 169 `Active`, 170 `Superseded`, 171 `Retired` |
| 172 | `Bundle` | 173 (one constructor) |
| 174 | `Registered` | 175 `Query`, 176 `Txn`, 177 `ImportedFn` |
| 178 | `Feed` | 179 (one constructor) |
| 180 | `FeedMember` | 181 (one constructor) |
| 182 | `PlanState` | 183 `Current`, 184 `Pinned`, 185 `Reverted`, 186 `Superseded` |
| 187 | `Plan` | 188 (one constructor) |
| 189 | `TzRelease` | 190 (one constructor) |
| 191 | `Zone` | 192 (one constructor) |
| 193 | `Setting` | 194 `StatementLimit`, 195 `TransactionLimit`, 196 `IdleLimit`, 197 `StreamIdleLimit`, 198 `RequestWindow`, 199 `SafePointDelay`, 200 `ReplicaWait`, 201 `DefaultLease`, 202 `FeedRetention`, 203 `StatsWindow`, 204 `StatsRetention`, 205 `WaitSampling`, 206 `ConflictLog`, 207 `VacuumBudget`, 208 `ScrubInterval`, 346 `LeaseGrace`, 347 `BlockingNoticePeriod`, 348 `ReplicaVacuumWait`, 451 `RetryLimit`, 470 `RetentionWindow` |
| 209 | `AuditEvent` | 210 `Login`, 211 `Grant`, 212 `Register`, 213 `Retire`, 214 `SchemaOp`, 215 `Call`, 216 `Denied`, 217 `Commit`, 218 `Truncate` |
| 349 | `ValueState` | 350 (one constructor) |
| 351 | `Role` | 352 (one constructor) |
| 353 | `Credential` | 354 (one constructor) |
| 355 | `GrantKind` | 356 `Log`, 357 `Deployment` |
| 358 | `RoleGrant` | 359 (one constructor) |
| 360 | `PeerMapping` | 361 (one constructor) |
| 373 | `CtorTarget` | 374 `InRelation`, 375 `OfType` |
| 376 | `Place` | 377 `Last`, 378 `Before`, 379 `After` |
| 380 | `Operator` | 381 `AddModule`, 382 `DropModule`, 383 `Install`, 384 `Uninstall`, 385 `AddType`, 386 `DropType`, 387 `AddRelation`, 388 `DropRelation`, 389 `AddSet`, 390 `DropSet`, 391 `AddConstructor`, 392 `RemoveConstructor`, 393 `Enable`, 394 `Disable`, 395 `Deprecate`, 396 `RenameConstructor`, 397 `RenameType`, 398 `RenameRelation`, 399 `RenameSet`, 400 `RenameField`, 401 `Split`, 402 `Merge`, 403 `Nest`, 404 `Unnest`, 405 `AddField`, 406 `DropField`, 407 `MoveField`, 408 `AddUnique`, 409 `DropUnique`, 410 `Rekey`, 411 `AddIndex`, 412 `DropIndex`, 413 `AddDefault`, 414 `DropDefault`, 415 `DefineFn`, 416 `RedefineFn`, 417 `DropFn`, 418 `DefineAggregate`, 419 `DropAggregate`, 420 `AddConstraint`, 421 `DropConstraint`, 422 `AddCheck`, 423 `DropCheck`, 424 `Store`, 425 `Unstore`, 426 `AddComputed`, 427 `RedefineComputed`, 428 `DropComputed`, 429 `Grant`, 430 `Revoke`, 431 `AdoptTzdb`, 432 `Script`, 433 `Revert`, 468 `AlterField` |
| 434 | `MigrationOp` | 435 (one constructor) |
| 436 | `ArmTarget` | 437 `OfFn`, 438 `OfConstraint`, 439 `OfComputed` |
| 440 | `ClauseBody` | 441 `Joins`, 442 `Member`, 443 `Arm`, 444 `Replacement`, 445 `Carry`, 446 `Fill`, 472 `Privilege` |
| 447 | `OpClause` | 448 (one constructor) |
| 452 | `Default` | 453 (one constructor) |
| 454 | `Computed` | 455 (one constructor) |
| 458 | `Term` | 459 (one constructor) |

Module `catalog`, relation variables:

| Id | Relation variable |
| --- | --- |
| 219 | `catalog::types` |
| 220 | `catalog::constructors` |
| 221 | `catalog::ctor_states` |
| 222 | `catalog::fields` |
| 223 | `catalog::relations` |
| 224 | `catalog::named_sets` |
| 225 | `catalog::set_members` |
| 226 | `catalog::indexes` |
| 227 | `catalog::index_ctors` |
| 228 | `catalog::constraints` |
| 229 | `catalog::functions` |
| 230 | `catalog::wasm_modules` |
| 231 | `catalog::extensions` |
| 232 | `catalog::migrations` |
| 233 | `catalog::bundles` |
| 234 | `catalog::registered` |
| 235 | `catalog::feeds` |
| 236 | `catalog::feed_members` |
| 237 | `catalog::plans` |
| 238 | `catalog::tz_releases` |
| 239 | `catalog::zones` |
| 240 | `catalog::settings` |
| 241 | `catalog::audit` |
| 242 | `catalog::credentials` |
| 362 | `catalog::value_states` |
| 363 | `catalog::roles` |
| 364 | `catalog::grants` |
| 365 | `catalog::peer_mappings` |
| 449 | `catalog::migration_ops` |
| 450 | `catalog::migration_clauses` |
| 456 | `catalog::defaults` |
| 457 | `catalog::computed` |

Module `sys`, types:

| Id | Type | Constructors |
| --- | --- | --- |
| 243 | `Collation` | 244 (one constructor) |
| 245 | `CtorStats` | 246 (one constructor) |
| 247 | `FieldStats` | 248 (one constructor) |
| 249 | `FieldValue` | 250 (one constructor) |
| 251 | `Fingerprint` | 252 (one constructor) |
| 253 | `QueryStats` | 254 (one constructor) |
| 255 | `QueryCtorStats` | 256 (one constructor) |
| 257 | `QueryWindow` | 258 (one constructor) |
| 259 | `SessionState` | 260 `Idle`, 261 `Busy`, 262 `InTransaction` |
| 263 | `Session` | 264 (one constructor) |
| 265 | `WaitReason` | 266 `StorageRead`, 267 `FileRead`, 268 `LogFlush`, 269 `Internal`, 270 `SafeSnapshot`, 271 `Inserter`, 272 `Admission` |
| 273 | `CallState` | 274 `Queued`, 275 `Running`, 276 `Waiting` |
| 277 | `Call` | 278 (one constructor) |
| 279 | `WaitSample` | 280 (one constructor) |
| 281 | `WaitTotal` | 282 (one constructor) |
| 283 | `Horizon` | 284 (one constructor) |
| 285 | `Conflict` | 286 (one constructor) |
| 291 | `Vacuum` | 292 (one constructor) |
| 293 | `JobKind` | 294 `Reindex`, 295 `Compact`, 296 `SchemaStage`, 297 `Verify` |
| 298 | `Job` | 299 (one constructor) |
| 300 | `PendingDrop` | 301 (one constructor) |
| 302 | `RequestMark` | 303 (one constructor) |
| 304 | `ProcessOption` | 305 (one constructor) |
| 310 | `RunId` | 311 (one constructor) |
| 312 | `Finding` | 313 `Checksum`, 314 `StalePage`, 315 `Order`, 316 `Orphan`, 317 `Missing`, 318 `Violation`, 473 `Structure` |
| 319 | `Format` | 320 (one constructor) |
| 366 | `CommitTime` | 367 (one constructor) |
| 368 | `SafeSnapshotMark` | 369 (one constructor) |

Module `sys`, relation variables:

| Id | Relation variable |
| --- | --- |
| 321 | `sys::collations` |
| 322 | `sys::ctor_stats` |
| 323 | `sys::field_stats` |
| 324 | `sys::field_values` |
| 325 | `sys::query_stats` |
| 326 | `sys::query_ctor_stats` |
| 327 | `sys::query_windows` |
| 328 | `sys::sessions` |
| 329 | `sys::calls` |
| 330 | `sys::wait_samples` |
| 331 | `sys::wait_totals` |
| 332 | `sys::horizons` |
| 333 | `sys::conflicts` |
| 334 | `sys::vacuums` |
| 335 | `sys::jobs` |
| 336 | `sys::pending_drops` |
| 337 | `sys::request_marks` |
| 338 | `sys::process_options` |
| 339 | `sys::verify_findings` |
| 340 | `sys::format` |
| 370 | `sys::commit_times` |
| 371 | `sys::safe_snapshots` |


Ids from 341 to 473 are listed in the rows of the types they belong
to. `Person` and the other types of examples are not engine types and
have no fixed id, nor has a `Principal` the schema declares
([security.md](security.md#11-the-principal-sum)). Ids 287 to 290, 306
to 309 and from 474 to 1023 are unassigned.

Fields and indexes have fixed ids in their own spaces. In edition
2026-10 and catalog version 1, the engine's original fields take ids from 0:
for each type of the tables above in id order, its header fields in
declared order, then each constructor's fields, constructors in listed
order and fields in declared order, excluding constructors with ids
from 471. They take ids 0 to 563:
`Option.Some.value` 0, `Result.Ok.value` 1, `Result.Err.error` 2,
`RejectReason.DumpDamaged.offset` 68, `RejectReason.Internal.what` 69,
`RejectReason.RowTooLarge`'s fields 70 to 73, the fields of the
`catalog` and `sys` modules from 92, and `Limit.Rounds.rounds` 563.
`RejectReason.KeyTooLarge` appends fields `relation`, `ctor`, `index`,
`size` and `limit` at ids 564 to 568 without changing any earlier id,
`ClauseBody.Privilege` its fields `privilege` and `target` at ids 569
and 570, and `Finding.Structure` its field `where` at id 571.
The engine's indexes take ids from 0: for
each relation variable of the tables above in id order, the index of
its key, then an index for each `unique` clause in written order;
they end at 60. A field or index that a later edition or catalog
version adds takes the next id of its space not used in any earlier
one. The engine declares no named sets, constraints, defaults or
computed fields. The prelude is not part of a schema: its functions
and aggregates have no catalog id and no row of `catalog::functions`.

Rulings: project/spec/errors/closed-sum, project/spec/errors/new-kind/coarse, project/spec/errors/new-kind/pinned, spec/evolution/concurrent-apply/refuse, spec/ids/engine-objects/fixed-table, spec/ids/prelude-fns/none, spec/limit-reason/limit-sum, spec/dump-damaged/offset, project/arch/errors/checker-guarantee/fail-statement, spec/row-too-large/refuse-at-write

## 5. Statistics

### 5.1 Constructor relation statistics

Statistics are kept per constructor relation: the rows of one
constructor of one relation variable. A number for a relation variable
or a constructor set is the sum of its constructor relations' numbers,
except distinct counts, which do not add (§5.2).

```adt
module sys

type CtorStats = { relation: Key catalog::relations, ctor: Key catalog::constructors,
                   live_rows: I64, dead_versions: I64, storage: Text,
                   last_vacuum: Option Instant, serialization_failures: I64,
                   quarantined: Bool }
relation ctor_stats : CtorStats volatile key (relation, ctor)
```

```adt
from h in sys::ctor_stats
where h.relation.name == "identities"
select { ctor: h.ctor.name, rows: h.live_rows, dead: h.dead_versions }
```

`live_rows` is an estimate maintained as writes commit; it is exact
only when no transaction is in progress on the constructor relation.
`dead_versions` counts the row versions of the constructor relation
that no snapshot can see and that vacuum has not yet removed.
`storage` describes the space the constructor relation occupies, in
implementation-defined text.

### 5.2 Field statistics

Statistics of field values are kept per constructor relation and
gathered online from its writes. The visible part:

```adt
type FieldStats = { relation: Key catalog::relations, ctor: Key catalog::constructors,
                    field: Key catalog::fields, distinct: F64, sampled: I64,
                    updated: Instant }
relation field_stats : FieldStats volatile key (relation, ctor, field)

type FieldValue = { relation: Key catalog::relations, ctor: Key catalog::constructors,
                    field: Key catalog::fields, value: Text, frequency: F64 }
relation field_values : FieldValue volatile key (relation, ctor, field, value)
```

`distinct` is the estimated number of distinct values of the field in
the constructor relation, and `sampled` the number of rows the
estimate is drawn from; `field_values` holds the most frequent values,
printed in the textual syntax
([interop.md](interop.md#1-textual-value-syntax)), with their estimated
frequency. Which values and how many are implementation-defined.

### 5.3 Query statistics

Execution statistics are keyed by the query's identity:

- a registered query or transaction by its catalog row, which names
  its bundle, and by the plan that ran;
- an ad hoc statement by a fingerprint of its type-checked form with
  literal parameters abstracted, so two texts differing only in
  constants share a row.

Every call has a fingerprint; registered calls also have `query`.

```adt
type Fingerprint = new Bytes

type QueryStats = { query: Option Key catalog::registered, fingerprint: Fingerprint,
                    plan: Option Key catalog::plans,
                    calls: I64, rows: I64, total: Duration, mean: Duration,
                    storage: Text, serialization_failures: I64, retries: I64 }
relation query_stats : QueryStats volatile key (query, fingerprint, plan)

type QueryCtorStats = { query: Option Key catalog::registered, fingerprint: Fingerprint,
                        plan: Option Key catalog::plans,
                        relation: Key catalog::relations, ctor: Key catalog::constructors,
                        rows_read: I64, pruned: I64 }
relation query_ctor_stats : QueryCtorStats volatile key (query, fingerprint, plan, relation, ctor)
```

A replan starts a new row, so a plan regression shows as two rows of
one query. `storage` describes the calls' reads of storage, in
implementation-defined text. `pruned` counts calls that did not read
the constructor relation although it is among the constructors the
statement reads ([queries.md §12.4](queries.md#124-the-constructors-a-statement-reads)); which calls those are is
implementation-defined.

### 5.4 Windowed history

Query statistics are also aggregated into time windows kept in the
database file for a retention period:

```adt
type QueryWindow = { query: Option Key catalog::registered, fingerprint: Fingerprint,
                     plan: Option Key catalog::plans, start: Instant, length: Duration,
                     calls: I64, rows: I64, mean: Duration, p99: Duration,
                     serialization_failures: I64 }
relation query_windows : QueryWindow volatile key (query, fingerprint, plan, start)
```

The window length and retention are the file settings `StatsWindow`
and `StatsRetention` (§10.2). A window is written when it closes; a
crash may lose the window open at the time. Windows older than the
retention are deleted by the engine. Rows survive restart, compaction
and replica promotion, and are not part of a dump.

```adt
query login_history (since: Instant) -> Set ({ start: Instant, plan: Option Key catalog::plans, p99: Duration }) =
  from w in sys::query_windows, q in catalog::registered
  where w.query == Some { value: key q } and q.name == "login_key" and w.start > since
  select { start: w.start, plan: w.plan, p99: w.p99 }
```

Rulings: exec/stats/per-ctor, ops/query-stats/by-name, ops/stats-history/windowed, ops/expose/relations, spec/catalog/physical/per-ctor-relation, spec/reads/static/static-types

## 6. Activity

### 6.1 Sessions and calls

```adt
type SessionState = Idle | Busy | InTransaction { txn: TxnId }
type Session = { id: Key sessions, principal: Text, bundle: Option Key catalog::bundles,
                 started: Instant, state: SessionState }
relation sessions : Session volatile key id

type WaitReason = StorageRead  { what: Text }
                | FileRead     { path: Text }
                | LogFlush
                | Internal     { what: Text }
                | SafeSnapshot
                | Inserter     { index: Text }
                | Admission
type CallState = Queued { since: Instant } | Running | Waiting { on: WaitReason }
type Call = { id: Key calls, session: Key sessions, query: Option Key catalog::registered,
              fingerprint: Fingerprint, started: Instant, state: CallState,
              priority: catalog::Priority, grant: I64, deadline: Option Instant }
relation calls : Call volatile key id
```

`WaitReason` is a closed sum. Its constructors:

| Constructor | The call waits for |
| --- | --- |
| `StorageRead` | data of the database file to be read; `what` is implementation-defined |
| `FileRead` | an external file ([interop.md](interop.md#3-file-scans)) |
| `LogFlush` | the log to be flushed at commit |
| `Internal` | a short-term wait inside the engine; `what` is implementation-defined |
| `SafeSnapshot` | a safe snapshot for a declared read-only transaction ([transactions.md](transactions.md#64-read-only-transactions)) |
| `Inserter` | a concurrent transaction inserting the same key into the named index to finish |
| `Admission` | admission (§8.1); the call's state is then `Queued` |

### 6.2 Sampled and cumulative waits

The state of every call is sampled at the interval of the
`WaitSampling` setting into a bounded history, and the time spent in
each wait reason is accumulated, both overall and per query:

```adt
type WaitSample = { at: Instant, call: Key calls, session: Key sessions,
                    query: Option Key catalog::registered, fingerprint: Fingerprint,
                    on: WaitReason }
relation wait_samples : WaitSample volatile key (at, call)

type WaitTotal = { reason: WaitReason, query: Option Key catalog::registered,
                   fingerprint: Option Fingerprint, count: I64, time: Duration }
relation wait_totals : WaitTotal volatile key (reason, query, fingerprint)
```

Samples older than the setting's `keep` are deleted. The row of
`wait_totals` with `fingerprint == None` is the total over every call.

### 6.3 Snapshot horizons

A row version a constructor relation no longer holds is kept while
some snapshot can see it. For each constructor relation, the engine
names the holder of the oldest snapshot that keeps such versions:

```adt
type Horizon = { relation: Key catalog::relations, ctor: Key catalog::constructors,
                 txn: TxnId, session: Key sessions, query: Option Key catalog::registered,
                 since: Instant }
relation horizons : Horizon volatile key (relation, ctor)
```

The engine never ends the holder on its own account; ending it is an
operator's `terminate` (§8.4) or a limit (§8.3).

### 6.4 Conflicts

Every serialization failure is logged in a bounded relation:

```adt
type Conflict = { aborted: TxnId, at: Instant, aborted_query: Option Key catalog::registered,
                  other: TxnId, other_query: Option Key catalog::registered,
                  relation: Key catalog::relations, ctor: Key catalog::constructors,
                  keys: Text, detail: Text }
relation conflicts : Conflict volatile key (aborted, at)
```

`relation` and `ctor` name the constructor relation of the
conflicting dependency, `keys` its key range in the textual syntax
(a range may be wider than the keys the transactions read or wrote),
and `detail` the dependency structure in implementation-defined text.
The log holds at most the `ConflictLog` setting's `rows`; older rows
are dropped first. Serialization failures are also counted per query
(`sys::query_stats`) and per constructor relation (`sys::ctor_stats`).

The error returned to the aborted call carries only its own
transaction and its own side (§4.2). The other transaction, its query
and the keys are readable only here, under §1.4.

```adt
query conflicts_by_ctor (since: Instant) -> Set ({ ctor: Text, n: I64 }) =
  from c in sys::conflicts
  where c.at > since
  group c by c.ctor into g
  select { ctor: g.key.name, n: count(x for x in g) }
```

### 6.5 Vacuum and jobs

```adt
type Vacuum = { id: Key vacuums, relation: Key catalog::relations,
                ctor: Key catalog::constructors, phase: Text, done: F64,
                dead_versions: I64, started: Instant }
relation vacuums : Vacuum volatile
  key id
  unique (relation, ctor)

type JobKind = Reindex | Compact | SchemaStage | Verify
type Job = { id: Key jobs, kind: JobKind, target: Text, phase: Text, done: F64, started: Instant }
relation jobs : Job volatile key id
```

Vacuum runs per relation variable and removes the dead versions of
its constructor relations (§11.5). `sys::vacuums` has a row per
constructor relation of a running vacuum: `dead_versions` is the
number it has left to remove, `phase` implementation-defined text.
`sys::jobs` has a row per running long operation of §11 and per staged
schema job ([evolution.md](evolution.md#34-staged-changes)). In both,
`done` is the fraction complete.

### 6.6 Pending drops and request marks

A dropped constraint, a dropped unique, a dropped check and the old
key of a relaxing `rekey` stay in force while a live bundle pinned
before the drop still relies on them
([constraints.md](constraints.md), [evolution.md](evolution.md#64-dropping-a-constraint)).
Each such pair of object and bundle is a row of `sys::pending_drops`,
and the report of the migration that drops the object names the same
rows:

```adt
type PendingDrop = { object: Text, dropped: Key catalog::migrations,
                     blocking: Key catalog::bundles }
relation pending_drops : PendingDrop volatile key (object, blocking)
```

The server deduplicates writes by request id per client
([wire.md](wire.md#65-idempotent-writes)). For each client it keeps the
recorded replies of committed requests within the `RequestWindow`
setting and a low-water mark below which ids are no longer recorded:

```adt
type RequestMark = { id: Key request_marks, client: Text, low: I64, recorded: I64 }
relation request_marks : RequestMark volatile
  key id
  unique (client)
```

`low` is the client's low-water mark and `recorded` the number of
replies kept for it.

Rulings: ops/waits/activity, ops/waits/sampled, ops/waits/cumulative, ops/limits/watermark/report, ops/conflicts/log, ops/conflicts/counters, ops/conflicts/leak/admin-only, ops/vacuum/progress, spec/catalog/physical/per-ctor-relation

## 7. Plans

### 7.1 The plan of a statement

A plan shows, for every `match` and every read of a sum, one arm per
constructor with its estimate, the constructors pruned and when (at
compile time from types, or at start from parameters), and each
union-index probe with the constructors its entries are routed to.

`explain` returns the plan of a query or write without running it.
Estimates come from the statistics of §5. With the running schema
extended by `index (user)` on `identities`:

```adt
explain from i in identities
        where i.user == users.key(3)
        select match i
          | EmailPass e -> e.email
          | Google g    -> g.gid
          | Apple a     -> a.sub
```

The result is one row `{ text: Text, document: Text }`. The `text`
form is for people and is not normative; for the query above it might
read:

```text
Probe identities (user)
  Route by constructor
    EmailPass  est 0.4  -> Project email
    Google     est 0.5  -> Project gid
    Apple      est 0.1  -> Project sub
```

### 7.2 Explain analyze

`explain analyze` runs the statement and adds to every plan node its
actual rows, loops and time, and its reads of storage in
implementation-defined text. For a write, it computes the
change set and checks every constraint on it, then discards the change
set: nothing is applied, and no effect of the write is visible to any
transaction. Its result adds the size of the change set per
constructor relation:

```adt
-- given from_gid: Text, to_gid: Text
explain analyze update g in identities.Google where g.gid == from_gid set Google { g with gid: to_gid }
-- { text: ..., document: ..., change: { identities.Google: -1 +1 }, constraints: checked }
```

`explain analyze` requires the privileges the statement itself
requires. The estimates and actual counts of a node are shown only
under §1.4.

### 7.3 Stored plans

Registered queries and transactions are planned at registration, and
their plans are stored in `catalog::plans`. The row whose `state` is
`Current` or `Pinned` is the plan every call runs; reading it shows
what calls run, not a fresh plan.

```adt
from p in catalog::plans
where p.query.name == "login_key" and p.query.bundle.name == "web-2.4"
  and (p.state == Current or p.state == Pinned)
select p.text
```

`explain` and `explain analyze` also take a registered call with its
arguments. The statement is checked and planned at the bundle's pinned
version, with its stored plan, exactly as a call of it runs, each
argument typed at its parameter's type at the pinned version
([applications.md §5](applications.md#5-binding-a-connection)):

```adt
explain analyze web-2.4::login_key(users.key(3))
```

A stored plan is replanned when a sum it reads changes version, and
when the statistics of a constructor relation it reads drift past an
implementation-defined threshold, as when a constructor grows from
empty. Each replan adds a row and marks the previous one
`Superseded`.

### 7.4 The plan document

`document` is a JSON text whose shape is versioned for tools:

- The top-level object has a member `"format"` naming the document
  version, a member `"release"` naming the engine release, and a
  member `"root"` holding the root node.
- Each node is an object with a member `"kind"` naming its operator,
  `"estimate"` its estimated rows, and `"children"` an array of nodes.
- A routing node has an array `"arms"` of objects, each with members
  `"ctor"` (the constructor id), `"name"` and `"child"`.
- Every read of a sum has an array `"pruned"` of objects with members
  `"ctor"`, `"name"` and `"stage"`, whose value is `"compile"` or
  `"start"`.
- Under `explain analyze`, each node has a member `"actual"` with
  members `"rows"`, `"loops"`, `"time_us"` and `"storage"`, the last
  an implementation-defined string.

Operator kinds and their other members change between engine
releases. A tool must ignore kinds and members it does not know. The
`format` changes only when a member listed above changes meaning.

### 7.5 Pinning

An operator may pin a stored plan:

```adt
pin plan catalog::plans.key(8812)
unpin plan catalog::plans.key(8812)
```

A pinned plan is kept across restarts and statistics replans. A schema
change that leaves every constructor relation and index the plan reads
in place, and every constructor set it reads unchanged, keeps it
pinned. Any other
schema change unpins it, replans, and records the reason in the
command result of the schema change and in the new plan's row.

A query of a newly registered bundle whose checked form equals that
of the same-named query of an earlier bundle of its family (§2.4)
inherits that query's current plan, and its pin if it had one, instead
of being planned afresh. Exactly one plan row of each registered query
is `Current` or `Pinned` at any time.

### 7.6 Reverting a regression

After a replan, whether caused by statistics, a schema change or a new
engine release, the engine compares the new plan's measured cost over
the windows of §5.4 with the previous plan's, when the previous plan
can still run. If the new
plan is worse by more than an implementation-defined margin, the engine
makes the previous plan current again and marks the new one
`Reverted`. A reverted plan is not chosen again until the next schema
change or a further statistics drift.

### 7.7 Plans across engine releases

Every stored plan records the engine release that made it. An engine
opening a file whose plans were made by another release must replan
every registered query and transaction before serving calls, against
the same schema and statistics. A pinned plan the new release cannot
reproduce is unpinned; the open reports it, and the new plan's row
records it.

Rulings: ops/explain/plan, ops/explain/analyze, ops/explain/stored, ops/explain/form/document, ops/plan-stability/pin, ops/plan-stability/last-good, ops/plan-release/stamp-replan, exec/replan/schema, exec/replan/stats

## 8. Resource control

### 8.1 Admission

Each call has a memory grant: the memory its plan needs, estimated at
registration for registered calls and at compile time for ad hoc ones,
and capped at the per-query bound of the `query_memory` open setting.
A call runs only when its grant fits: the sum of the grants of running
calls must never exceed the query memory budget.

Calls that do not fit wait in one queue ordered by priority, highest
first, then by the start of the call's transaction, oldest first. When
grants are released, the head of the queue runs if its grant fits.
Priorities are the values of `Priority`: registered calls run at their
bundle's priority, ad hoc calls at `AdHoc`, the lowest. A call or a
`Begin` may state a lower priority for itself, never a higher one
([wire.md](wire.md#51-call)). The engine bounds
the queue, in calls and in the bytes its calls hold while they wait,
and answers a call that finds it full with `Retry { reason:
Overloaded {..} }` ([wire.md](wire.md#8-admission-and-overload)). A
call that alone holds more than the queue's whole byte bound is
answered at once with `Rejected { reason: StatementLimit { limit:
Memory { bytes } } }`, `bytes` the byte bound, never with `Retry`.
The bytes a call holds are the memory of its arguments, of the text
and parameters of ad hoc text, and of a fixed overhead per call, as
the call is admitted; the batches of an Append that arrive after it
is admitted are not counted.

A running call that needs more memory than its grant spills first to
pages of the pool within the grant, then, when the pool has none to
spare, to spill files in the spill directory, encrypted as
[security.md](security.md#91-what-is-encrypted) states. A grant never
grows while its call runs. A call that can stay within its grant
neither in the pool nor by spilling, as a hash build larger than the
grant with no spill space to spare, is rejected with `StatementLimit {
limit: Memory { bytes } }`, `bytes` its grant.

### 8.2 Bundle quotas

A bundle may declare its priority and a budget in its header:

```adt
bundle batch-1.2 pins 9 (5d0e...) edition "2026-10" priority background budget { memory: 0.25, workers: 2 }
```

- `priority` is `background` or `regular` (the default).
- `memory` is the largest fraction of the query memory budget the
  grants of the bundle's running calls may hold together; a call that
  would exceed it waits in the queue.
- `workers` is the largest number of worker threads the bundle's
  calls may occupy at once.

### 8.3 Limits and deadlines

| Limit | Bounds | On expiry | Error |
| --- | --- | --- | --- |
| statement | one query or write execution | the statement fails; an interactive transaction may continue or roll back | `Rejected { reason: StatementLimit { limit: Time {..} } }` |
| transaction | an interactive transaction from begin to commit | the transaction aborts | `Rejected { reason: TransactionExpired {..} }` |
| idle | an interactive transaction with no call running | the transaction aborts; the session continues | `Rejected { reason: TransactionExpired { idle: true, .. } }` |
| deadline | one call, as an absolute instant sent by the caller | the call fails; a registered transaction aborts | `Rejected { reason: DeadlineExceeded }` |

A registered transaction is one call: it is bounded by the statement
limit and its deadline, not by the transaction limit. The statement,
transaction and idle limits are set per file by the settings of §10.2.
A call or a `Begin` may carry its own statement limit and deadline
(§10.4); the server reports the transaction and idle limits in force
when the connection is ready ([wire.md](wire.md#34-binding-and-ready)).
A registered query or transaction may declare its own statement
limit, which replaces the file and call values for its calls:

```adt
type Order = { id: Key orders, month: Date, region: Text, total: Decimal 2 }
relation orders : Order key id generate sequence
query report (m: Date) limit 30s -> Set ({ region: Text, total: Decimal 2 }) =
  from o in orders where o.month == m
  group o by o.region into g
  select { region: g.key, total: sum(x.total for x in g) }
```

The declared limit is part of the registered signature and changes only
with the bundle. A deadline applies in addition to every limit; the
call stops at whichever comes first. Deadlines travel with each call
([wire.md](wire.md#5-calls)).

### 8.4 Cancel and terminate

- `cancel call c` stops the call `c` promptly, without letting it run
  to the end of its statement. The call fails with `Rejected { reason: Cancelled }`. An interactive
  transaction the call belongs to stays open and may continue or roll
  back; a registered transaction, which is one call, aborts.
- `terminate session s` aborts every open transaction of the session,
  fails each of its running calls with `Rejected { reason: Terminated
  }`, and ends the session.
- In process, a call may be given a cancellation token; cancelling the
  token from another thread has the effect of `cancel call`
  ([embedding.md](embedding.md#7-cancellation)). Over the server, a call
  is cancelled in band ([wire.md](wire.md#57-cancel)).

A session may always cancel its own calls. Cancelling or terminating
another session's needs the `cancel` capability.

Rulings: ops/admission/queue, ops/admission/grant, ops/quota/priority, ops/quota/budget, ops/limits/statement, ops/limits/transaction, ops/limits/idle, ops/limits/deadline, ops/limits/declared, spec/limit-reason/limit-sum, ops/cancel/cancel, ops/cancel/terminate, ops/cancel/token

## 9. Operational commands

### 9.1 Commands

Operations that are effects rather than data are commands: statements
beside queries and writes. A command is checked like any statement and
returns a relation describing what it did. Its values read no relation.
A command is not a write:
it does not belong to a transaction, an abort does not undo it, and it
must not appear inside a `transact` block, a registered query or
transaction, a function or a constraint.

| Command | Effect | Capability |
| --- | --- | --- |
| `explain q`, `explain analyze s` | §7.1, §7.2 | as for the statement |
| `explain [analyze] B::q(args)` | §7.3 | `call` on the query, and `monitor` |
| `pin plan p`, `unpin plan p` | §7.5 | `maintain` |
| `cancel call c` | §8.4 | own calls, or `cancel` |
| `terminate session s` | §8.4 | `cancel` |
| `vacuum r` | §11.5 | `maintain` |
| `verify`, `verify r` | §11.1 | `maintain` |
| `reindex r`, `reindex r index (f) [in S]` | §11.4 | `maintain` |
| `compact` | §11.3 | `maintain` |
| `finalize upgrade` | §12.2 | `maintain` |
| `set setting S = v` | §10.2 | `configure` |
| `retire bundle B` | [applications.md](applications.md#43-retirement) | the bundle's owner or the schema owner |
| `supersede bundle B by B2` | [applications.md](applications.md#44-supersession) | `B`'s owner or the schema owner |
| `export (q) to "path"` | [interop.md](interop.md#51-export) | `write_files`, through a server |
| `register bundle "file"` | [applications.md](applications.md#41-registration) | as registration requires |
| `apply migrations "file" ...` | [evolution.md](evolution.md#23-applying-a-migration-set) | `schema` |
| `print schema` | [applications.md](applications.md#9-the-printed-schema) | none beyond reading the catalog |
| `archive audit through n to "path"` | [security.md](security.md#8-audit) | `maintain` and `write_files` |
| `rotate key kek "uri"`, `rotate key dek` | [security.md](security.md#94-rotation) | `configure` |
| `destroy key dek n` | [security.md](security.md#94-rotation) | `configure` |
| `dump to "path" [encrypt key]` | [interop.md](interop.md#6-dump-and-restore) | `maintain` and `read` on every relation dumped, and `write_files` through a server; the security section only with `schema` |
| `restore from "source" to "path" [before version n \| at instant t \| at position p]` | [interop.md](interop.md#6-dump-and-restore) | in process or on a closed file only |
| `backup to "target" [incremental]` | [interop.md](interop.md#71-backup) | `maintain` |

Every command is available as an ad hoc statement over the server
protocol and as a call of the in-process API; the command line tool
(§9.4) wraps them. `restore` writes a new file, so it never runs
against a database being served.

`retire bundle` and `supersede bundle` are commands, never operators
in a migration.

### 9.2 Results

Each command returns a set of records:

| Command | Result rows |
| --- | --- |
| `explain`, `explain analyze` | §7.1, §7.2 |
| `pin plan`, `unpin plan` | the plan's row of `catalog::plans` |
| `cancel call`, `terminate session` | `{ call: Key sys::calls }` for each call stopped |
| `vacuum` | `{ ctor: Key catalog::constructors, removed: I64, freed: Text }` per constructor relation, `freed` describing the space freed in implementation-defined text |
| `verify` | the rows of `sys::verify_findings` it wrote |
| `reindex`, `compact` | `{ job: Key sys::jobs }`, returned when the job starts |
| `finalize upgrade` | the row of `sys::format` |
| `set setting` | the new row of `catalog::settings` |
| `retire bundle`, `supersede bundle` | the row of `catalog::bundles` of the bundle retired or superseded |
| `export` | `{ path: Text, rows: I64 }` |
| `register bundle` | the bundle's row of `catalog::bundles` |
| `apply migrations` | one row per migration applied, of `catalog::migrations`, with the migration report |
| `print schema` | `{ text: Text }` |
| `archive audit` | `{ through: I64, digest: Bytes }` |
| `rotate key`, `destroy key` | `{ version: I64 }` |
| `dump`, `backup` | the manifest, as one `{ manifest: Text }` row |
| `restore` | the manifest of the restored file |

### 9.3 Capabilities

Operations need these privileges of the one privilege list of
[security.md](security.md#3-grants), granted to roles independently:

| Privilege | Allows |
| --- | --- |
| `monitor` | reading `sys::` relations and plans, within §1.4 |
| `cancel` | cancelling and terminating other sessions' calls |
| `maintain` | `vacuum`, `verify`, `reindex`, `compact`, plan pinning, `finalize upgrade` |
| `configure` | `set setting` |
| `read_files` | reading files through a server, within `file_roots` ([interop.md](interop.md#3-file-scans)) |
| `write_files` | `export` through a server, within `export_roots` ([interop.md](interop.md#51-export)) |

```adt
grant monitor, cancel to oncall
grant maintain to dba
```

A caller of the in-process API holds every privilege. Commands are
issued as ad hoc statements, so a principal issuing one over the server
also needs `adhoc` ([applications.md](applications.md#7-the-ad-hoc-role)).

### 9.4 The command line tool

One tool, `adtb`, works on a closed database file and on a running
server. Given a file, it opens it as the one process that owns it, and
fails if another process does. Given a server address, it connects as
a client and issues commands and statements.

```sh
$ adtb repl app.adt
$ adtb repl adt://db.internal:7000 --role dba
$ adtb verify app.adt
$ adtb compact app.adt --into app.compact.adt
$ adtb explain app.adt 'web-2.4::login_key(users.key(3))'
$ adtb upgrade app.adt --finalize
$ adtb serve app.adt --pool 8GiB
$ adtb register app.adt bundles/web.adb
$ adtb migrate app.adt migrations/0013
$ adtb dump app.adt app.dump
```

Every command of §9.1 is available through the tool. Operations that
need the file closed, such as an offline compaction (§11.3) or a
restore ([interop.md](interop.md#6-dump-and-restore)), are available
only on a file.

Rulings: ops/admin/in-language, ops/admin/actions/commands, ops/admin/cli, ops/admin/role/capabilities

## 10. Configuration

### 10.1 Fixed behaviour, derived sizes

Engine behaviour is fixed: there is no setting that changes what a
program means. Settings exist only for resource sizes, retention
periods and limits. Every resource size has a default derived from the
host (memory size and core count) when the file is opened; an override
replaces the derived value. Each setting belongs to one of four scopes,
by what it must agree with:

| Scope | Agrees with | Set | Changes | Shown in |
| --- | --- | --- | --- | --- |
| format | the data | when the file is created | only by compaction into a new file | `sys::format` |
| file | every process opening the file | `catalog::settings` | by `set setting` | `catalog::settings` |
| open | the host and process | when the file is opened | only by reopening | `sys::process_options` |
| call | the caller | per call or `Begin` | per call | the call's row in `sys::calls` |

### 10.2 File settings

File settings are a catalog relation, read by queries and changed by
the `set setting` command, which needs `configure`. A change is
atomic and logged; settings move with the file to another host
and reach replicas through the log.

```adt
type Setting { id: Key settings }
  = StatementLimit   { limit: Option Duration }
  | TransactionLimit { limit: Option Duration }
  | IdleLimit        { limit: Option Duration }
  | StreamIdleLimit  { limit: Duration }
  | RequestWindow    { keep: Duration }
  | SafePointDelay   { delay: Duration }
  | ReplicaWait      { wait: Duration }
  | DefaultLease     { lease: Duration }
  | FeedRetention    { keep: Duration }
  | StatsWindow      { length: Duration }
  | StatsRetention   { keep: Duration }
  | WaitSampling     { every: Duration, keep: Duration }
  | ConflictLog      { rows: I64 }
  | VacuumBudget     { io: F64, cpu: F64 }
  | ScrubInterval    { every: Option Duration }
  | LeaseGrace       { grace: Duration }
  | BlockingNoticePeriod { period: Duration }
  | ReplicaVacuumWait    { wait: Duration }
  | RetryLimit           { retries: I64 }
  | RetentionWindow      { length: Duration }
relation catalog::settings : Setting key id
```

`catalog::settings` holds exactly one row of each constructor. `set
setting` replaces the row of the named constructor with the given
payload:

```adt
set setting StatementLimit = { limit: Some { value: 5s } }
set setting VacuumBudget = { io: 0.2, cpu: 0.5 }
```

A limit of `None` is no limit. A new value applies to calls that start
after the `set setting` command completes. Listener peer mappings are
not settings: they are rows of `catalog::peer_mappings` (§2.6),
changed by `map peer` and `unmap peer`
([security.md](security.md#22-sources)); listener addresses are open
settings.

| Setting | Governs | Default |
| --- | --- | --- |
| `StatementLimit` | each query or write execution (§8.3) | `None` |
| `TransactionLimit` | an interactive transaction from begin to commit (§8.3) | `Some { value: 5s }` |
| `IdleLimit` | an interactive transaction with no call running (§8.3) | `None` |
| `StreamIdleLimit` | a result stream left without credit ([wire.md](wire.md#55-credit)) | `30s` |
| `RequestWindow` | how long request ids are remembered for deduplication ([wire.md](wire.md#65-idempotent-writes)) | `24h` |
| `RetryLimit` | the most reruns of a registered transaction, of ad hoc text that writes and of a `transact` block run in process after a serialization failure, when the call or block states no `retry n`: at most `retries + 1` attempts ([transactions.md §6.2](transactions.md#62-retryable-units)) | `5` |
| `SafePointDelay` | the bound in the promise that safe snapshots keep forming while read-write transactions run, and the most a read-write transaction may start later than it is issued for it ([replication.md](replication.md#12-reads-on-a-replica)) | `100ms` |
| `ReplicaWait` | the longest a replica read given a position waits for a safe point before `Retry { reason: SafePointTimeout {..} }` | `5s` |
| `DefaultLease` | the lease of a bundle whose header states none ([applications.md](applications.md#42-leases)) | `30d` |
| `LeaseGrace` | how long after a lease's expiry, computed from its last renewal in the log, the primary still treats it as live, covering clock skew; a replica refuses the bundle from expiry ([applications.md](applications.md#42-leases)) | `1h` |
| `BlockingNoticePeriod` | how long after a refused migration or revoke the engine sends `BlockingMigration` to the sessions of each blocking bundle (§4.5) | `24h` |
| `ReplicaVacuumWait` | the longest a replica's log replay waits for a read whose snapshot sees a row version the replay would remove, before cancelling it with `Retry { reason: ReplayConflict {..} }` ([replication.md](replication.md#12-reads-on-a-replica)) | `30s` |
| `FeedRetention` | how long the log is kept for change streams ([interop.md](interop.md#52-change-streams)) | `7d` |
| `StatsWindow` | the length of a query statistics window (§5.4) | `1min` |
| `StatsRetention` | how long windows are kept (§5.4) | `30d` |
| `WaitSampling` | the sampling interval and retention of wait samples (§6.2) | `every: 1s`, `keep` equal to `StatsRetention` |
| `ConflictLog` | rows kept in `sys::conflicts` (§6.4) | `10000` |
| `VacuumBudget` | the fractions of device bandwidth and of CPU that vacuum may use (§11.5) | `io: 0.1`, `cpu: 1.0` |
| `ScrubInterval` | the period of the background scrub (§11.1) | `Some { value: 7d }` |
| `RetentionWindow` | how far back reads of the past may reach: the engine keeps every row version a snapshot within `length` of now can see ([derived.md](derived.md#61-the-retention-window)). At `0s` it keeps none beyond what running snapshots see, and every `at` block whose state no running snapshot reads is rejected with `HistoryUnavailable`. A new length applies to the versions vacuum has not yet reclaimed | `0s` |

### 10.3 Open settings

Open settings are given when the file is opened, through the
in-process API or the tool's options, and are fixed for the life of
the process. They include the page pool size (by default 25% of the
host's memory), the number of worker threads (by default one per
core), the query memory budget and per-query bound (`query_memory`),
the I/O path, the log archive destination, `file_roots` (the
directories a server may read files from, empty by default;
[interop.md](interop.md#3-file-scans)), `export_roots` (the directories
a server may export into, empty by default;
[interop.md](interop.md#51-export)), the telemetry endpoint and trace
sampling (§13). They are shown read-only in a volatile relation,
with whether each value was derived or given:

```adt
type ProcessOption = { id: Key process_options, name: Text, value: Text, derived: Bool }
relation sys::process_options : ProcessOption volatile
  key id
  unique (name)
```

```rust
let db = adtb::open("app.adt", &schema::LOG, &[&queries::BUNDLE],
                     OpenOptions { pool_bytes: Some(8 << 30), workers: Some(16), ..OpenOptions::default() })?;
```

### 10.4 Format and session settings

Format settings are fixed when the file is created and recorded in its
header: the page size and the checksum or encryption algorithm. The
page size is a power of two from 4 KiB to 64 KiB, 4 KiB by default;
every page of the file has that size. They
change only by compacting into a new file (§11.3) that states new
values. `sys::format` shows them (§12.2).

Per-call settings are the statement limit, the deadline and the
priority. A call or a `Begin` may carry each of them
([wire.md](wire.md#5-calls), [embedding.md](embedding.md#4-calls)); a
statement limit there overrides the file setting for that call or
transaction, and a priority may only be lower than the bundle's class
(§8.1). The transaction and idle limits are file settings only; the
server reports them when a connection is ready.

Rulings: ops/config/derived, ops/config/home/relation, ops/config/home/process/split, ops/config/scope/format, ops/config/scope/file, ops/config/scope/open, ops/config/scope/session

## 11. Integrity and maintenance

The engine promises crash safety and detection of corrupted pages of
the data file. Its fault model is crash stop, torn writes of any write
larger than the device's atomic unit, bit rot, latent sector errors,
misdirected writes and lost writes. A page that fails its checksum or
tag is never returned as data. A lost write is detected during
recovery, and by `verify` and the background scrub, which compare each
page's log position with the retained log; a lost write whose log has
left the retained log, on a page not read by recovery, is not
detected.

### 11.1 Verification

`verify` checks the whole database; `verify r` checks the constructor
relations and indexes of relation variable `r` and every constraint
that reads it. It runs at a snapshot while the database serves calls,
and checks:

1. Structure: every page checksum, the structural invariants of every
   index, among them the order of its entries, and the version chains
   of every heap.
2. Cross-reference: every live row version has its entry in every
   index covering its constructor, and every index entry names an
   existing row version of the constructor relation it names. A
   constructor outside an index's constructor set, or a removed
   constructor, has no entries in it.
3. Logic: every admitted constraint, run once as its denial query over
   the whole database; every nonempty result is a finding.

A free page holds nothing, and neither check reads it as damage. A
background scrub reads every page of the file over each
`ScrubInterval`, so a latent checksum failure is found before a call
needs the page. Scrub findings go to the same relation.

```adt
type RunId = new I64

type Finding { id: Key verify_findings, run: RunId, at: Instant }
  = Checksum  { where: Text }
  | StalePage { where: Text }
  | Order     { index: Key catalog::indexes, where: Text }
  | Orphan    { index: Key catalog::indexes, ctor: Key catalog::constructors, where: Text }
  | Missing   { relation: Key catalog::relations, ctor: Key catalog::constructors,
                index: Key catalog::indexes, where: Text }
  | Violation { constraint: Key catalog::constraints, witness: Text }
  | Structure { where: Text }
relation sys::verify_findings : Finding volatile key id
```

`where` locates the finding in storage, in implementation-defined
text. `StalePage` is a lost write. `witness` is the first violating row
in the textual syntax. Findings are kept in the file. A run replaces only the
findings of the last run over its scope: the whole database, one
relation variable, or the scrub.

On a quarantined database, `verify` writes no findings to the file. It
answers at least one `Structure` row naming the quarantine. When the
catalog cannot be read it checks pages and the log only, and the row says
so, and `verify r` fails with `Quarantined`.

### 11.2 Quarantine

A corrupted page, detected by checksum or by its log position on any
read, quarantines what holds it: one constructor relation, or one
index, which holds entries of every constructor of its set. A page no
relation holds, such as one of the commit log or the catalog,
quarantines the database: every call fails with `Fatal { reason:
Quarantined {..} }`, and the file opens for `verify` and `dump` only. An
`Order`, `Orphan` or `Missing` finding (§11.1) quarantines the index it
names. A file with neither copy of its header intact, or a log that
recovery refuses, does not open.

A call fails with `Fatal { reason: Quarantined {..} }` exactly when
the constructors it reads
([queries.md §12.4](queries.md#124-the-constructors-a-statement-reads))
include a quarantined constructor relation. Every other call
continues. A write is refused in the same way where its constraint or
reference checks must read a quarantined constructor relation. A key
spans its sum, so a write to any constructor of a sum with a quarantined
constructor is refused, whatever the key value:

```adt
-- identities.Apple quarantined
from i in identities.(EmailPass | Google) select i.id    -- runs
from i in identities select i.id                         -- fails
```

A transaction that wrote a constructor relation such a statement could
not now write fails its commit with `Quarantined` and leaves nothing
visible. A read-only transaction commits.

The call that finds the damage fails with `Quarantined`, whichever it
reads. The rules below hold from the next call.

A quarantined index never fails a read: queries do not use it, so
only their cost changes. A statement that writes a constructor
relation the index covers
([queries.md §12.4](queries.md#124-the-constructors-a-statement-reads))
fails with `Fatal { reason: Quarantined {..} }`:

```adt
-- given u: Key users
-- index (user) in identities.(Google | Apple) quarantined
from i in identities.Google where i.user == u select i.gid   -- runs
insert identities.Google { user: u, gid: "g-2" }             -- fails
```

A quarantined index is restored by `reindex` (§11.4). A quarantined
constructor relation is restored by restoring a backup to a new file
and swapping that file in with the database closed
([interop.md](interop.md#7-backup-and-point-in-time-restore)); commits
past the point restored are lost;
`sys::ctor_stats.quarantined` shows the state.

### 11.3 Compaction

Freed pages are reused, and the file shrinks only by compaction, which
writes every live version into a new file:

- Offline, with the file closed, through the tool:
  `$ adtb compact app.adt --into app.compact.adt`. This is also the way to
  change a format setting: `--page-size 16KiB`.
- Online, by the `compact` command, while the database serves calls:
  the engine copies the database at a snapshot into the new file,
  catches up with the commits made since, and switches to the new file
  at a moment when no call is between statements. Both files exist
  for the duration.

Either way the new file holds the same relations, catalog, constructor
ids, key values and schema version, and needs free space equal to the
live data. Online compaction runs as a job in `sys::jobs`. Physical
replicas and incremental backups start from a new base after a
compaction ([replication.md](replication.md#11-physical-replicas)).

### 11.4 Reindex

`reindex r` rebuilds every index of relation variable `r`, its key index
and uniques included; `reindex r index (user) in (Google | Apple)`
rebuilds the one index with that declaration clause. `in S` may be left
out when one index of `r` has those fields; with several it is
`Invalid`. A union index is rebuilt whole, over every constructor of its
set.

- `online`, the default: the new index is built while writes
  continue, catches up with the writes made meanwhile, and replaces
  the old one, as a staged job.
- `offline`: writes to every constructor in the index's set wait until
  the rebuild ends.

```adt
-- with:
--   relation identities : Identity key id generate uuidv7 unique (email) in EmailPass unique (gid) in Google unique (sub) in Apple index (user) in (Google | Apple)
reindex identities index (user) in (Google | Apple) online
```

A rebuild that fails, as on two versions of one unique key that one
snapshot sees, keeps the old index and any quarantine, and writes a
`Structure` finding naming the index. The finding stays until the next
rebuild of that index, and no `verify` run replaces it. Closing the
database stops a running rebuild and keeps the old index.

### 11.5 Vacuum

Background vacuum runs per relation variable when its dead versions
justify it, by an implementation-defined measure. A dead version a
snapshot on a replica can still see is not removed
([replication.md](replication.md#12-reads-on-a-replica)). An operator may start
one at once with `vacuum r`. All vacuum work, background or started,
runs within the `VacuumBudget` setting. Progress is in `sys::vacuums`
(§6.5).

Rulings: ops/check/structural, ops/check/cross, ops/check/logical, ops/check/scrub, ops/halt-scope/heap, spec/quarantine/verify/yes, spec/quarantine/unowned/database, ops/compact/offline-copy, ops/compact/online-copy, ops/reindex/online, ops/reindex/offline, ops/vacuum/trigger, ops/vacuum/throttle, buffer/faults/corrupt-detect, buffer/faults/stale-page/lsn-check, spec/catalog/physical/per-ctor-relation, spec/reads/static/static-types, spec/quarantine/index/cost-only

## 12. Releases and compatibility

### 12.1 Promises

An engine release makes four promises:

1. File: it reads and writes every database file any earlier release
   wrote, subject to §12.2.
2. Protocol: a client and a server interoperate when their supported
   protocol version ranges overlap; the client offers its minimum and
   maximum and the server picks one or refuses with `Rejected {
   reason: UnsupportedProtocol {..} }` ([wire.md](wire.md#131-negotiation)).
   Each release states the range it supports.
3. Editions: it compiles every edition of the language, and each
   bundle is checked under the edition it names. A breaking change to
   the language is a new edition.
4. Catalog: the types of system relations change only through catalog
   operators in the catalog log, which bundles pin by its version and
   which apply readers-first
   ([evolution.md](evolution.md#10-engine-releases)). The engine's own
   enabling of catalog constructors it added is deployment state, not
   a catalog version. A registered
   query over the catalog or a `sys::` relation keeps compiling and
   running across releases.

### 12.2 Upgrades and downgrade

A new release opening a file written by an older one writes only
formats the previous release reads, until an operator finalizes the
upgrade:

```sh
$ adtb upgrade app.adt --finalize
finalize upgrade
```

A release reads every format of the data file and the log that any
earlier release wrote. Until then the file can be reopened by the
previous release, so trying a release is reversible; features that need a newer format are
unavailable, and the release's catalog operators are not applied: the
catalog keeps the version the previous release wrote. Finalizing
applies the release's catalog operators to the catalog log, records
the new format in the file header, and cannot be undone. When
readers-first refuses one of those catalog operators, `finalize
upgrade` is refused with a report naming the bundles that block it:
nothing is applied, the upgrade stays unfinalized and the previous
release can still open the file. Nothing is
appended to the schema log by an upgrade or by an open. In process,
`Database::finalize_upgrade()` does the same
([embedding.md](embedding.md)).

Replicas upgrade to a new release before the primary: a primary writes
a new format only after every replica reports the new release
([replication.md](replication.md#3-schema-leases-and-upgrades-at-replicas)).

```adt
type Format = { id: Key format, file: Text, page_size: I64, format: I64, readable_by: Text,
                finalized: Bool, release: Text }
relation sys::format : Format volatile key id
```

Rulings: ops/promise/file, ops/promise/protocol, ops/promise/editions, ops/promise/catalog, ops/protocol/negotiate, ops/catalog-release/catalog-smo, ops/downgrade/finalize, repl/upgrade/replicas-first

## 13. Telemetry

### 13.1 Metrics

Every numeric field of the `sys::` relations of §5 and §6 is also
available outside the language:

- A server exports them as OpenTelemetry metrics to the endpoint given
  by the open settings, named `adtb.` followed by the relation name and
  field (`adtb.ctor_stats.dead_versions`), with the row's key fields as
  attributes, printed by name (`relation`, `ctor`, `bundle`, `query`).
- The in-process API returns the same values as a snapshot through a
  callback ([embedding.md](embedding.md#13-metrics)), for the host to
  forward.

The visibility rule of §1.4 does not apply to these exports: whoever
configures the endpoint or holds the in-process handle sees every
value.

### 13.2 Traces

A server traces in OpenTelemetry form:

- One span per execution. The span of a registered call is named by
  the query's name; its attributes are `db.system.name` = `adtb`,
  `db.namespace` (the database), `db.query.summary` (the query name),
  `db.query.text` (the registered text), `adtb.bundle`, `adtb.plan`
  and `adtb.arms_pruned`. The span of an ad hoc statement is named by
  its fingerprint and does not record its text.
- Child spans for planning or compilation, each pipeline, the commit
  check, the log flush, and each wait, named by the `WaitReason`
  constructor.
- A caller's W3C Trace Context sent with a call
  ([wire.md](wire.md#5-calls)) is the parent of the call's span.

Which executions are traced is decided by the trace sampling open
setting.

```text
span "login_key"  db.system.name=adtb  db.namespace=app
                  db.query.summary=login_key  adtb.bundle=web-2.4  adtb.arms_pruned=Apple
  span "plan"
  span "pipeline 1"
  span "LogFlush"
```

Rulings: ops/expose/otel, ops/expose/callback, ops/tracing/query-span, ops/tracing/operator-spans, ops/tracing/propagate

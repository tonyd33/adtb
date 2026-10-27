# Schema evolution

This part defines how a schema changes. The schema of a database is
the fold of an ordered log of typed operators; a position in the log
is a schema version. It covers migrations, the operators and their
inverses, how a migration is applied and refused, how constraints,
grants and definitions move with the data, the lifecycle of a
constructor (declared, enabled, deprecated, removed), the
readers-first rule, how clients pinned to an older version keep
running through translation, how a change meets running transactions,
how schema changes reach replicas, and how engine releases change the
catalog's own sums.

Bundles, registration, leases and retirement are deployment state,
defined in [applications.md](applications.md); this part uses bundles
as readers.

## Example histories

Examples in each section are independent histories unless the section
says otherwise, and each states the version it starts from. Unless a
section says otherwise, it starts from version `10 (a91c...)`, the
start of the history in [README.md](README.md#conventions), whose
schema is:

```adt fragment
module auth
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
set OAuth of identities = Google
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
constraint login_present =
  from i in identities
  where match i
    | EmailPass e -> scalar_len(e.email) == 0
    | Google g    -> scalar_len(g.gid) == 0
  select i
```

This part extends it with badges:

```adt
type Badge { name: Text } = Plain | Color { rgb: I32 }
relation post_badges : Badge key name
relation user_badges : Badge key name
set Simple of post_badges = Plain
```

Version `11` is `11 (c7d2...)` and version `12` is `12 (c41e...)` in
the history of §4.2 and §4.3.

## 1. The operator log

### 1.1 Migrations

The authored schema artifact is the log. A migration is a file of
operators with a number; migrations are numbered from 1 without gaps.
Applying migration `n` to a database at version `n - 1` produces
version `n`. A migration applies as one schema change: either every
operator in it takes effect, or none does (§3).

```adt fragment
-- 0011, from version 10
module auth
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
  arm display_name: Apple a -> a.sub
  arm login_present: Apple a -> scalar_len(a.sub) == 0
add unique (sub) in identities.Apple
```

This is the only migration `0011` of the history; every later section
that shows `0011` shows this text.

`0011 add ...` is shorthand for a migration file `0011` holding the one
operator.

Operators are written by the developer. The engine never infers
operators from a difference between two schemas, and no form exists
that states a whole schema and asks the engine for the operators that
reach it.

The schema at version `n` is the result of applying the operators of
migrations 1 to `n` in order to the empty schema. Every schema object
(modules, types, relation variables, keys, uniques, indexes, named
constructor sets, schema functions, aggregates, constraints, stored
views, defaults, computed fields, grants, constructor write states,
installed extensions, the zone rules release) exists only because an
operator in the log created it. The engine prints the folded schema
for review ([applications.md §9](applications.md#9-the-printed-schema)).

### 1.2 What the log holds

The log holds only migrations written by their author. Changes to data
shape, to definitions and to constructor write states are all
operators in it. In particular `define fn`, `redefine fn`, `drop fn`,
`add constraint`, `drop constraint`, `enable`, `disable`, `deprecate`
and `revert` are operators: each changes what a version means, and each produces a
version when its migration applies. A migration may mix them, so an
`add constructor` and the redefinitions it requires land in one
version and are rechecked together.

The engine never appends to the log. Bundle registration, leases,
supersession and retirement are deployment state in
`catalog::bundles` ([applications.md §4](applications.md#4-bundle-lifecycle)),
not operators. The engine's own catalog sums change through a separate
catalog log (§10).

### 1.3 Operators act on types

An operator that changes a type changes every relation variable of
that type. `add constructor Badge.Emoji { glyph: Text }` gives both
`post_badges` and `user_badges` an `Emoji` constructor relation.
Clauses of the operator that concern one relation variable name it:
`joins post_badges.Simple` puts `Emoji` in the named set `Simple` of
`post_badges` and in no set of `user_badges`. A type whose key field is
typed `Key r` belongs to the one relation variable `r`
([model.md](model.md#7-keys)).

An operator on a parametric type reaches every instantiation of it:
adding a constructor to `type Result a e` adds it in every relation
variable and field whose type is any `Result`, and every match over
any `Result` is rechecked. The constructor has one id, shared by every
instantiation.

### 1.4 Modules

Every migration file starts with a `module` header naming the module
it belongs to. An operator names its object by its qualified name when
the object belongs to another module (`auth::Identity.Apple`).

A database has one operator log for all its modules. Migration numbers
are database-wide: migrations of different modules share one sequence.

Rulings: surface/source/op-log, surface/migration/handwritten, evolution/in-log, evolution/log-ops, evolution/smo, evolution/types, model/param-evolution/live, surface/namespace/modules

## 2. Versions

### 2.1 Position and hash

A schema version is identified by its position in the log and by a
hash of the log up to that position. It is written as the position
followed by the hash: `10 (a91c...)`. The hash is BLAKE3-256, chained:

```text
h(n) = BLAKE3(h(n - 1) ++ encoded(n))
```

where `encoded(n)` is the byte encoding of migration `n`'s resolved
operators, its rows of `catalog::migration_ops` and
`catalog::migration_clauses`
([catalog.md §2.4](catalog.md#24-versions-bundles-and-registered-calls)),
in the wire value encoding ([wire.md §4](wire.md#4-value-encoding)):
the operator count as a `uvarint`, then for each operator in ordinal
order its `Operator` value, the count of its clauses as a `uvarint`
and each clause's `ClauseBody` value in clause order. A resolved operator
names every object it touches by its id. The printed text of a
migration is not hashed. Bytes an operator carries (Wasm modules and
extension packages for `install`, zone rule data for `adopt tzdb`) are
stored in the log; the operator's value holds the BLAKE3 hash of each,
so the version hash covers them. `h(0)` is 32 zero bytes. Two
databases agree on a version's hash exactly when their logs agree up
to it.

People are shown the hash truncated to its first 4 bytes in hex; the
wire and every check carry all 32 bytes. Every place a version is
stated for checking (a bundle header, a pinned past read, a migration
set being applied) states both; the engine compares the hash, refuses
a mismatch, and reports the position. Source text writes the hash as
all 64 hex digits; a shorter hash is refused. Examples in this
specification elide all but the first 4 bytes as `a91c...`.

### 2.2 Linear history

The log is a chain. A database has exactly one migration with each
number. Two branches of development that each write a migration onto
version `7` cannot both be numbered `8`: the branch applied second must
renumber its migration to `9` and recheck it against the schema at `8`.
There is no merge of migrations.

### 2.3 Applying a migration set

A deploy presents an ordered set of migration files with the command
`apply migrations "file" ...`, which needs `schema`
([catalog.md](catalog.md#9-operational-commands)); the CLI and the
embedding API wrap it. The engine:

1. refuses the whole set, before any migration of it applies, when its
   numbers have a gap, or when the first file numbered above the
   current version is not numbered the current version plus one;
2. compares each file numbered at or below the current version with
   the log, by hash, and refuses the whole set at the first mismatch,
   naming the number (the log has diverged);
3. applies each remaining file in order, each as its own schema change
   (§3);
4. stops at the first refused migration; versions already produced
   remain.

While a staged migration is pending (§3.4), applying a migration is
refused with `Rejected { reason: MigrationInProgress }`.

### 2.4 Versions are never reused

The log only grows. No operator removes a migration from the log or
renumbers one. Undoing a migration appends its inverse as a new
migration (§5.4).

Rulings: surface/version/counter, surface/both, surface/version/history/linear, evolution/revert, spec/evolution/version-hash/value-encoding, spec/evolution/migration-gap/whole-set, spec/evolution/concurrent-apply/refuse, spec/evolution/operator-type/catalog-sum

## 3. Applying a migration

### 3.1 One transaction

A migration without staged operators applies in one transaction, the
applying transaction. A migration with staged operators applies as the
staged migration of §3.4, whose final commit produces the version. Either
way its operators and any data writes it makes (§4.14) take effect
together or not at all. A migration that is refused or fails leaves the
schema at the previous version, the log unchanged and the data as it
was.

A migration is one statement of the applying transaction. It denotes
the composite of its operators' data maps, in order. A script's data
map applies its change set, evaluated on the state the operators before
it produce, at their schema (§4.14). Every constraint of the final
schema holds after the migration, checked once, at its end; an added
constraint is validated on that final state, so a script may repair
data that a constraint added earlier in the same migration needs.

The applying transaction is serializable like any other
([transactions.md](transactions.md#1-serializability)): data writes in
it conflict with concurrent transactions by the ordinary rules. Every
precondition of an operator (that a constructor is empty, that a
constraint is satisfied, that no stored value holds a constructor) is
a predicate read of the applying transaction
([semantics.md §8.2](semantics.md#82-items-reads-and-writes)).

### 3.2 Checks at apply

Before the version commits, the engine checks it. The migration is
refused if any of the following fails, and the refusal report lists
every failure, not only the first:

| Check | Refused when |
| --- | --- |
| Typing | an operator does not type against the schema produced by the operators before it in the migration, except that a reference type in a declaration may name a relation variable a later operator of the same migration adds, as stated below |
| Recheck of schema objects | a schema function, constraint, stored view, default or computed field no longer type checks or is no longer exhaustive at the new version |
| Constraint carrying | a constraint touched by a structural operator, including one whose drop is pending, has no image under it (§6.2, §6.4) |
| Constraint validation | an added constraint has violating rows in the migration's final state (§6.3) |
| Admission | a constraint or stored view at the new version has no index-bounded incremental check ([constraints.md](constraints.md#5-admission), [derived.md §2.2](derived.md#22-admission)) |
| Grants | a merge needs grants it does not state, or a bundle owner no longer holds the grants a live bundle uses (§6.5) |
| Live bundles | a query or transaction of a live bundle cannot be served at its pinned version across the migration (§7.2) |
| Readers-first | an `enable` has a reader that forbids it (§8.1) |
| Lifecycle | a `remove` meets a constructor that is not deprecated or not empty (§4.4); a `disable` meets one that has rows or a live writer, or a `deprecate` one that is not enabled (§4.3); a `drop field` meets a field of an old key whose drop is pending (§4.10) |
| Row size | a constructor relation whose smallest row's stored form is larger than the engine's row limit, named in the report ([writes.md §1.6](writes.md#16-rows-too-large)) |
| Code identity | a host function is redefined while a live bundle pins a version with a different code for it (§7.1) |
| Privilege | the principal may not append the operators ([security.md](security.md#3-grants)) |

A reference cycle is declared within one migration: each type of the
cycle may name, as `Key r`, a relation variable added after it. When
the type of that relation variable is added in the same migration, a
reference may also select its constructors, as `Key r.C` or `Key
r.(C | S)`: the constructors and named sets it names resolve when the
migration's last operator has applied, so a named set added later in
the migration may be named too, and a sum may reference its own
constructors in the operator that adds it. A selection that names no
constructor or named set of `r` at the end of the migration fails the
typing check. Any other name, including a type named by a field, must
be declared by an operator before the one that uses it.

```adt fragment
-- 0001
module world
add module world
add type Region { id: Key regions, parent: Option (Key regions.Settled) }
  = Wild
  | Town { mayor: Key people.Living }
add type Person { id: Key people, home: Key regions.Town }
  = Living
  | Dead
add relation regions : Region key id generate sequence
add set Settled of regions = Town
add relation people : Person key id generate sequence
```

A schema file whose declarations each name only declarations before
them, apart from the relation variables of reference types, is a valid
first migration of an empty database file when each declaration is
prefixed by its verb (`add`; `define` for a function or an aggregate;
nothing for `store`), and the migration produces the schema the file
declares.

A failure of a schema object always refuses the migration: schema
objects exist only at the current version, so the migration must
itself redefine or drop what it breaks. A failure of a live bundle
refuses the migration, and nothing waits: the report names each
blocking bundle, and the migration applies only when presented again
after the bundle is released (retired, superseded or expired) or with
changed operators
([applications.md §6](applications.md#6-what-a-schema-change-does-to-bundles)).
For the setting `BlockingNoticePeriod` (24 hours by default) after
the refusal, the engine sends the notice `BlockingMigration { bundle,
version }` to every session bound to a blocking bundle
([catalog.md](catalog.md#45-notices)). A migration never retires a
bundle. No call of a live bundle fails
with a type, exhaustiveness or translation error because of a
migration; constraint violations of its translated writes are ordinary
errors (§6.3).

The migration report also names every constraint drop that is pending
and the bundles holding it (§6.4).

### 3.3 Atomicity as observed

Every transaction runs against exactly one schema version: the
version current at its snapshot, or the pinned version of the bundle
whose calls it makes. A transaction whose snapshot precedes
the commit of version `n` runs at version `n - 1` to its end,
including its writes. A transaction whose snapshot follows it runs at
`n`. No transaction observes part of a migration.

Ad hoc text runs at the schema and catalog versions of the
transaction's snapshot, and a registered call at its bundle's pinned
version and pinned catalog version. Every statement of a transaction
runs at one schema version and one catalog version, so a call that
would run at another is refused with
`Rejected { reason: MixedVersions }`: in particular ad hoc text in a
transaction that has made a registered call, and a registered call in
one that has run ad hoc text
([transactions.md §2](transactions.md#2-statements)).

Every statement reads the schema version it runs at as an item, and
the commit that produces version `n` writes it
([semantics.md §8.2](semantics.md#82-items-reads-and-writes)). A
read-write transaction at `n - 1` that commits after version `n` is
therefore serialized against the migration by the ordinary rules
([transactions.md](transactions.md#1-serializability)).

### 3.4 Staged changes

Some operators require work proportional to the data: building an
index or unique, validating an added constraint or key, moving rows
for a split or merge, rebuilding a stored view. A migration holding
one is staged: it is pending from the moment it is presented until it
commits version `n` or fails.

A staged migration is equivalent to one applying transaction
serialized at its final commit, which commits version `n` and appends
the migration to the log. Every check of §3.2 holds at that commit.
Two things beyond that transaction are observable: while the migration
is pending, its progress is a row of a system relation
([catalog.md](catalog.md#1-system-relations)), and a failure has a
report. Until version `n` commits, the current version stays `n - 1`
and every other transaction runs at it. A write committed while the
migration is pending that violates a constraint or unique of version
`n` fails the migration.

A staged migration that cannot complete (a new unique finds a
duplicate key, a validation finds a violating row) is discarded: the
log, the schema and the data are as they were, and the report names
the operator and the rows that caused the failure. No inverse is
appended.

### 3.5 Pinned versions

A transaction runs at the schema version of its snapshot, and a
connection bound to a bundle runs at the bundle's pinned version
([applications.md §5](applications.md#5-binding-a-connection)). When
that version is older than the current version, its reads and writes
are translated through the operators between the two (§7). A staged
migration runs while bundles pinned earlier keep being served.

Rulings: engine/transactional-ddl, engine/online-states, engine/ddl-online-txn/staged-job, engine/pinned-version, evolution/breaks/recompile, evolution/breaks/invisible/registered, surface/registration/on-change/refuse, spec/schema-siread/version-item, spec/evolution/migration-statement/one-statement, spec/evolution/preconditions/predicate-read, spec/evolution/staged-meaning/one-transaction, spec/txn/one-version/one-pair, spec/evolution/forward-refs/later-key, spec/evolution/forward-sets/same-migration, spec/row-too-large/refuse-at-write

## 4. Operators

Each operator is a verb, an object and clauses. The table lists them
with their inverses; §5 states what an inverse is.

| Operator | Inverse | Exact |
| --- | --- | --- |
| `add module m` | `drop module m` | yes |
| `drop module m` | `add module m` | yes; refused unless `m` holds nothing |
| `install m "v" from "file"` | `uninstall m` | yes |
| `uninstall m` | `install` of the same package | yes; refused while anything outside `m` uses it |
| `add type T = ...` | `drop type T` | yes |
| `drop type T` | `add type T = ...` | yes; refused unless no relation variable or field uses `T` |
| `add relation r : T ...` | `drop relation r` | yes |
| `drop relation r` | `add relation r : T ...` | yes; refused unless `r` is empty |
| `add set S of r = ...`, `drop set S of r` | each other | yes |
| `add constructor T.C { ... }` | `remove constructor T.C`, removing the arms its `arm` clauses added | yes |
| `remove constructor T.C` | `add constructor T.C { ... }` with a fresh id | yes (applies only to an empty constructor) |
| `enable r.C`, `enable T.C` | `disable` of it from declared, `deprecate` of it from deprecated | yes |
| `disable r.C`, `disable T.C` | `enable r.C`, `enable T.C` | yes; refused while any row of `r.C`, or any stored value of `T.C`, exists |
| `deprecate r.C`, `deprecate T.C` | `enable r.C`, `enable T.C` | yes; applies only to an enabled constructor |
| `rename constructor T.C to D` | `rename constructor T.D to C` | yes |
| `rename type T to U` | `rename type U to T` | yes |
| `rename relation r to s` | `rename relation s to r` | yes |
| `rename set S of r to S2` | `rename set S2 of r to S` | yes |
| `rename field T.C.f to g` | `rename field T.C.g to f` | yes |
| `split constructor T.C into ... by match ...` | `merge` back into `C` | yes when the routing is invertible (§4.6) |
| `merge constructor T.(C \| D) into M { ... }` | `split constructor T.M` by its discriminator | yes when its `by match` is invertible (§4.7) |
| `nest constructor T.(C \| D) into N { f: S }` | `unnest constructor T.N` | yes |
| `unnest constructor T.N` | `nest` | yes |
| `add field T.C.f: U fill e` | `drop field T.C.f` | yes |
| `alter field T.C.f on delete a` | `alter field T.C.f` to the previous action | yes |
| `drop field T.C.f` | `add field T.C.f: U fill e` | no: values of `f` are lost |
| `move field T.C.f to T fill e` | `move field T.f to T.C` | yes for rows of `C` (§4.9) |
| `move field T.f to T.(C \| D)` | `move field ... to T fill e` | no for the other constructors |
| `add unique ...`, `drop unique ...` | each other | yes; a drop is held pending (§6.4) |
| `rekey relation r key (...)` | `rekey` to the old key | yes; a relaxing rekey is held pending (§6.4) |
| `add index ...`, `drop index ...` | each other | yes |
| `add default ...`, `drop default ...` | each other | yes |
| `define fn`, `drop fn` | each other | yes |
| `redefine fn f ...` | `redefine fn f` with the previous body | yes |
| `define aggregate`, `drop aggregate` | each other | yes ([interop.md §10](interop.md#10-user-defined-aggregates)) |
| `add constraint`, `add check` | `drop constraint`, `drop check` | yes |
| `drop constraint`, `drop check` | `add constraint`, `add check` | yes, validated as an add (§6.3); held pending (§6.4) |
| `store f`, `unstore f` | each other | yes ([derived.md §2](derived.md#2-stored-views)) |
| `add computed`, `drop computed` | each other | yes ([derived.md §5](derived.md#5-computed-fields)) |
| `redefine computed` | `redefine computed` with the previous expression | yes |
| `grant ...`, `revoke ...` on schema relations, constructor sets and `call fn` | each other | yes ([security.md](security.md#3-grants)) |
| `adopt tzdb "r" from "file"` | `adopt tzdb` of the previous release | yes ([catalog.md §3.1](catalog.md#31-zone-rules)) |
| `script ...` | none | no |
| `revert n` | `revert` of the reverting migration | as its parts |

### 4.1 Modules, types, relations and sets

`add module m` creates an empty module; `drop module m` removes one,
and is refused while the module holds any type, relation variable,
set, definition or constraint. `install m "v" from "file"` installs an
extension module with its definitions and code
([interop.md §9.6](interop.md#96-extensions)); `uninstall m` removes
it, and is refused while any definition outside `m` names it.

`add type`, `add relation` and `add set` declare, with the declaration
syntax of [syntax.md](syntax.md#5-declarations) after the verb:

```adt
-- 0101, from a version 100 of a separate history with the same schema
module auth
add type Device = { id: Key devices, owner: Key users on delete restrict, label: Text }
add relation devices : Device key id generate sequence
add set Mail of identities = EmailPass
```

`drop relation r` is refused while `r` has rows or while a reference
type, schema function, constraint, stored view or live bundle names
`r`. `drop type T` is refused while any relation variable, field or
definition uses `T`.

### 4.2 Add a constructor

```adt fragment
-- 0011, from version 10
module auth
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
  arm display_name: Apple a -> a.sub
  arm login_present: Apple a -> scalar_len(a.sub) == 0
add unique (sub) in identities.Apple
```

The operator adds constructor `Apple` to type `Identity` and an empty
`Apple` constructor relation to every relation variable of
`Identity`. Ids are a function of the log, assigned as
[catalog.md §1.2](catalog.md#12-identity-of-catalog-objects) states;
the products of split, merge, nest and unnest, and a constructor
re-added by an inverse, are new constructors. Ids are never reused: a constructor removed
and added again takes a new id. Its name may be reused: a constructor
name freed by a split or a remove may name a new
constructor, and documents that name constructors map names through
the log ([interop.md](interop.md)).

A record is a sum with one constructor named after the type
([model.md §2.2](model.md#22-records)), and `add constructor` applies
to it as to any sum: `add constructor Point.Polar { r: F64, theta: F64 }`
makes `Point` the sum `Point { x: I64, y: I64 } | Polar { r: F64, theta: F64 }`,
the record's fields remaining those of its constructor `Point`. Every
schema object is rechecked (§3.2), and so is every registered query
and transaction of a live bundle; the migration is refused if one
fails.

Clauses:

- `joins r.S, ...` names, for each relation variable, the named
  constructor sets the new constructor joins. A named set not listed
  does not gain it. Every index, unique and reference type ranging
  over the set gains it; one written as an explicit constructor list
  (`identities.(EmailPass | Google)`) never gains it. An index or
  unique written without `in` ranges over every constructor and gains
  it.
- `before C`, `after C` place the new constructor in the type's
  declaration order, which is the order of an ordered sum
  ([model.md §10](model.md#10-order)). Without either, the constructor
  is appended last.
- `arm name: P -> e` gives the new constructor's arm in the schema
  function, computed field (`r.f`) or match-scoped constraint `name`.
  Its target is the match in `name` whose scrutinee's constructor set
  contains the new constructor at the new version. The clause is
  refused unless exactly one match in `name` has such a scrutinee; the
  definition must then be redefined with `redefine fn`, or the
  constraint dropped and added, in the same migration. Arms are given
  only for schema objects; registered queries of bundles keep their
  pinned version (§7). The inverse of an `arm` clause
  removes the arm it added and restores any arm it replaced.

A schema function, computed field or constraint with a match whose
scrutinee's constructor set contains the new constructor, neither given
an arm nor redefined in the migration, fails the recheck and refuses the migration (§3.2). An added arm of a
match-scoped constraint is checked against the new constructor
relation, which is empty.

The new constructor starts in the declared state: it is part of the
type, queries at the new version must cover it, and every insert or
retag into it is refused. On a sum used as a field type, every write
that stores a value of it in a field is refused. `enable` makes it writable (§4.3, §8).

### 4.3 Enable and deprecate

```adt fragment
-- 0012, continuing §4.2
module auth
enable identities.Apple
```

```adt fragment
-- 0019, later in the same history
module auth
deprecate identities.Apple
```

A constructor relation is in one of three write states:

| State | Reached by | Inserts and retags into it | Reads, updates within it, deletes, retags out of it |
| --- | --- | --- | --- |
| declared | `add constructor`; `disable` | refused | allowed |
| enabled | `enable`; `add type` | allowed | allowed |
| deprecated | `deprecate` | refused | allowed |

`enable` moves a declared or deprecated constructor to enabled; its
inverse is `disable` when the constructor was declared and `deprecate`
when it was deprecated. `disable` moves an enabled constructor back to
declared; it is refused while any row of the constructor
relation exists, while a live bundle at any version has a query or
transaction that can write into it, or while a delete action in the
schema retags into it. `deprecate` applies only to an enabled
constructor.

Constructors created by `add type` start enabled. A relation variable
added after its type gained a constructor takes, for that constructor,
the constructor's state in its type, kept in `catalog::value_states`:
`add relation r : T` after `add constructor T.C`, with no `enable T.C`
between them, starts `r.C` declared. Products of `split`
and `unnest` take their source's state, per relation variable,
deprecated included. Products of `merge` and `nest` start enabled when
every source constructor is enabled, and declared when some source is
declared; merging or nesting a deprecated constructor is refused.

The states are per relation variable, recorded in
`catalog::ctor_states` keyed by relation variable and constructor
([catalog.md](catalog.md#21-types-and-constructors)): `enable
post_badges.Emoji` leaves `user_badges.Emoji` declared. A refused insert or retag is
`Rejected { reason: NotOpen {..} }` naming the constructor relation and its state.

A constructor of a sum used as a field type has the same three states,
kept per type in `catalog::value_states`
([catalog.md](catalog.md#21-types-and-constructors)); its operators
name the type: `enable Provider.Okta`. Its state gates storing a value
of it. An insert, update or retag that stores, in any field at any
depth, a value of a declared or deprecated constructor is refused with
`Rejected { reason: NotOpen {..} }` naming the constructor and its
state, unless the row with the same key before the write holds an
equal value at the same field path. Reads are allowed in every state.
`disable T.C` is refused while any stored field value holds `C`.

`enable` is subject to readers-first (§8.1). `deprecate` is refused
while a live bundle at any version has a query or transaction that can
write into the constructor relation, or while a delete action in the
schema retags into it (§8.2).

### 4.4 Remove a constructor

```adt fragment
-- 0020, continuing §4.3: Apple is deprecated and empty at version 19
module auth
remove constructor Identity.Apple
redefine fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
```

`remove` never deletes data. It is refused unless, at the version
before it:

1. the constructor is deprecated in every relation variable of the
   type, by a migration earlier than this one;
2. every constructor relation of it is empty;
3. no reference type names a constructor set that contains it, other
   than the whole sum;
4. no named set would become empty.

For a constructor of a sum used as a field type, conditions 1 and 2
read: the constructor is deprecated in its type by an earlier
migration, and no stored field value holds it.

The operator removes the constructor from every named set, drops every
index and unique whose constructor set is exactly that constructor,
narrows every other index and unique, drops grants on it (§6.5), and
carries constraints (§6.2). Schema functions that name it fail the
recheck unless redefined in the same migration; a `remove` that is the
inverse of an `add constructor` also removes the arms that operator's
`arm` clauses added.

A populated constructor goes away in one of two ways, each step with
its own inverse:

- a `merge` with other constructors (§4.7), which replaces it, so no
  `remove` follows; or
- `deprecate` it, so no new row can enter it; then move its rows out
  with a `script` that retags them (§4.14); then `remove` it in a later
  migration.

### 4.5 Rename

```adt fragment
-- 0031, from a version 30 at which Identity also has Facebook { fid: Text }
module auth
rename constructor Identity.Facebook to Meta
rename type Identity to Login
rename relation identities to logins
rename set OAuth of logins to Federated
rename field Login.EmailPass.hash to pw_hash
```

A rename changes a name and nothing else. A renamed constructor keeps
its catalog id, its rows and its wire tag; a renamed relation variable
keeps its rows, keys, indexes, constraints and grants; a renamed field
keeps its values and grants. Every schema object naming the renamed
object is rewritten. A rename is refused when the new name is already
taken in its scope (the module for types and relations, the relation
variable for sets, the constructor or header for fields). Bundles
pinned before the rename keep the old name at their version (§7).

### 4.6 Split a constructor

```adt fragment
-- 0041, from a version 40 at which
--   type Provider = Google | Apple
--   type Identity { id: Key identities, user: Key users }
--     = EmailPass { email: Text, hash: Bytes } | OAuth { provider: Provider, sub: Text }
--   relation identities : Identity key id generate uuidv7 unique (provider, sub) in OAuth
--   and display_name matching EmailPass and OAuth
module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }
  unique (gid) in Google
  unique (sub) in Apple
  arm display_name: Google g -> g.gid
  arm display_name: Apple a -> a.sub
```

The operator replaces constructor `OAuth` by the listed constructors,
which receive new ids (§4.2), take the source's position in the
declaration order in listed order, and start in the write state of
§4.3. The `match` after `by` has no
scrutinee: it matches a row of the constructor being split. It is type
checked and checked exhaustive like any match, so every row goes to
exactly one new constructor; an arm whose result is not one of the new
constructors is a type error. The routing reads only the row: it may
not range over relations or call functions that do. A catch-all is a
final unguarded arm; there is no separate catch-all form.

Each row keeps its key across the split. Header fields are unchanged:
routing arms may omit the key and header fields, which carry over.
Indexes and uniques over `OAuth` are re-scoped by the routing: each
field the routing carries whole into a new constructor is indexed
there under its new name. A unique or index naming a field that no new
constructor keeps (here `provider`) is refused unless the split states
its replacements, as `unique` and `index` clauses after the routing.
A reference type `Key r.OAuth` is retyped to `Key r.(Google | Apple)`,
and so is a selection of `OAuth` in a query or row type of a schema
object: `from i in identities.OAuth` reads `identities.(Google |
Apple)`. A field of a product typed by a reference to the split
constructor is retyped the same way, so a constructor that references
itself is split with products that reference one another; a product may
instead name its target among the products, as `Key r.Google`, resolved
at the new version. The routing is typed with every reference to the
split constructor retyped: a field it carries whole keeps its value,
which now references one of the products.
Constraints are carried (§6.2) and grants re-scoped (§6.5).

```adt fragment
-- from a version at which
--   type Place { id: Key places } = Location { parent: Option (Key places.Location), scale: Scale } | Faction
--   type Scale = Realm | Town
split constructor Place.Location
  into Region { parent: Option (Key places.Location) }
     | Settlement { parent: Option (Key places.Location) }
  by match
    | Location { parent, scale: Realm } -> Region { parent: parent }
    | Location { parent, scale: Town } -> Settlement { parent: parent }
-- Place = Region { parent: Option (Key places.(Region | Settlement)) }
--       | Settlement { parent: Option (Key places.(Region | Settlement)) } | Faction
```

A split takes `arm` clauses as a merge does (§4.7): for every schema
function, computed field and match-scoped constraint whose match names
the split constructor, one clause per new constructor, which replace
its arm. Without them that dependent fails the recheck and the split
is refused (§3.2).

The routing is invertible when each arm's pattern binds every field of
the old constructor, or fixes it to a literal or a nullary constructor, and its result uses each
bound variable exactly once as a whole field value with no other
computation, and every product is the result of some arm. A field
access `g.f` on a constructor binder `g` counts as binding field `f`
and carrying it whole. The engine then derives the inverse merge. A
split whose routing is not invertible is a declared loss and has no
inverse: it is
refused while a live bundle pinned before it reads or writes the old
constructor (§7.2), and a `revert` of it is refused. A refusal for a
product that no arm reaches names that product.

### 4.7 Merge constructors

```adt
-- 0013, from version 12 (§4.3)
module auth
add type Provider = Google | Apple
merge constructor Identity.(Google | Apple) into Social
  { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }
  arm display_name: Social s -> s.sub
  arm login_present: Social s -> scalar_len(s.sub) == 0
```

The operator replaces the listed constructors by one new constructor
with a new id, which takes the position in the declaration order of
the first source in that order. The new constructor has exactly one discriminator
field: the field whose type is a declared nullary sum whose
constructors are exactly the merged constructors' names (`Provider`).
The merge fills it with each row's old constructor. A merge whose new
constructor has no such field, or more than one, is refused.
The `by match` clause maps each old row to the new value, and must not
compute the discriminator otherwise than as its own constructor name.
Without `by`, each old constructor's fields map by name and type to the
new constructor's other fields, and a field with no counterpart is
refused. Routing arms may omit the key and header fields, which carry
over.

Like `add constructor`, a merge takes an `arm name: P -> e` clause
(§4.2) for every schema function, computed field and match-scoped
constraint whose match names the merged constructors. The clause's arm,
over the new constructor, replaces their arms. A dependent with no
such clause, not redefined in the same migration, fails the recheck and
refuses the merge (§3.2).

Uniques re-scope with the discriminator: `unique (gid) in Google` and
`unique (sub) in Apple` become `unique (provider, sub) in Social`.
Every named set containing all merged constructors contains the new
one; a named set containing some but not all of them is refused, and
so is a reference type naming such a set. A reference type naming
exactly the merged set (`Key identities.(Google | Apple)`) is retyped
to `Key identities.Social`. Grants follow §6.5.

A merge is exact when its `by match` is invertible by the condition of
§4.6: each arm binds every field of its source constructor or fixes it
to a literal or a nullary constructor, and carries each bound variable
whole into one field, where `g.gid` of the binder `g` carries field
`gid` whole. Its inverse is then the split routed by the
discriminator. A merge whose `by match` computes or drops fields is a
declared loss, with no inverse, under the rules of §5.2.

### 4.8 Nest and unnest

```adt
-- 0013, from version 12 (§4.3)
module auth
add type OAuthLogin = Google { gid: Text } | Apple { sub: Text }
nest constructor Identity.(Google | Apple) into Social { login: OAuthLogin }
  arm display_name: Social s -> match s.login | Google g -> g.gid | Apple a -> a.sub
  arm login_present: Social s ->
    match s.login | Google g -> scalar_len(g.gid) == 0 | Apple a -> scalar_len(a.sub) == 0
```

`nest` replaces the listed constructors by one new constructor whose
single field holds a value of an existing sum with constructors of the
same names and fields. The new constructor takes the position in the
declaration order of the first nested constructor in that order. Each
row becomes the new constructor holding its old value, under the same
key. Uniques and indexes re-scope by path:
`unique (gid) in Google` becomes `unique (login.Google.gid) in Social`.
Matches over the outer type change shape and are rechecked: a nest
takes `arm` clauses as a merge does (§4.7), one for every schema
function, computed field and match-scoped constraint whose match names
the nested constructors, and is refused at the recheck without them. A
reference type naming a set that contains some but not all nested
constructors is refused.

`unnest constructor Identity.Social` is the inverse: the value-level
constructors of the field become relation-level constructors with new
ids, which take the source's position in the declaration order, in
the order of the field's sum. An unnest takes `arm` clauses as a split does (§4.6), one per new
constructor for every dependent whose match names the unnested
constructor, and is refused at the recheck without them.

### 4.9 Fields

```adt
-- 0013 to 0015, from version 12 (§4.3)
-- 0013
module auth
add field Identity.EmailPass.verified: Bool fill false
-- 0014
module auth
move field Identity.EmailPass.verified to Identity fill match
  | Google g -> true
  | Apple a  -> true
-- 0015
module auth
drop field Identity.EmailPass.hash
```

- `add field T.C.f: U fill e` adds a field to a constructor (or to the
  header, `T.f`). A reference field states its delete action after its
  type, as in a declaration: `add field T.C.f: Option (Key r) on delete
  clear fill None`; without one it is `restrict`. `e` gives the value for every existing row; it may
  read the row being filled and nothing else. The fill is recorded with
  the operator: a write translated from an earlier version that lacks
  the field takes the fill when the relation has no default for it
  (§7.4). The fill is not a default for later inserts; defaults are
  separate ([derived.md §4](derived.md#4-defaults)).
- `drop field T.C.f` removes the field and its values. It is a declared
  loss: it is refused while a live bundle pinned before it reads the
  field, and its inverse is an `add field` whose fill the reverting
  migration must state.
- `move field T.C.f to T fill e` moves a field from a constructor to
  the header. `e` is a match whose arms are exactly the constructors
  that lack the field; it gives their value. Its inverse moves the
  field back down to `C` and drops it from the others.
- `move field T.f to T.(C | D)` moves a header field down to the listed
  constructors and drops it from the others, a declared loss for them.
- `alter field T.C.f on delete a` changes the delete action of a
  reference field (or of a header field, `T.f`) to `a`. Rows are
  unchanged; the action governs deletes from the version it produces.
  It is refused unless the field's type is a `Key` type or an `Option`
  of one, under the same rules as a declared action. Its inverse is
  `alter field` back to the field's previous action, `restrict` when it
  stated none.

### 4.10 Keys, uniques and indexes

`add unique (f) in r.C`, `drop unique (f) in r.C`, `add index (f) in
r.S`, `drop index (f) in r.S`, and `interval index` likewise, change
the relation variable's clauses. Adding a unique validates the data
(§6.3) and runs as a staged change (§3.4). Dropping one is held pending
like a dropped constraint (§6.4). Indexes do not change what
any query returns.

`unique` may range over deterministic expressions of the row's fields
as well as fields: `add unique (lower(email)) in identities.EmailPass`
([derived.md §5.2](derived.md#52-expression-indexes)).

`rekey relation r key (f, ...)` changes the key of relation variable
`r`:

```adt fragment
-- 0051, from a version 50 at which
--   type Price = { id: Key prices, sku: Text, during: Span Instant, amount: Decimal 2 }
--   relation prices : Price key id generate sequence
--   type Order { id: Key orders } = Line { price: Key prices, qty: I32 } | Note { text: Text }
--   relation orders : Order key id generate sequence
module billing
rekey relation prices key (sku, during)
  carry Order.Line.price
drop field Price.id
```

The new key is validated against the existing rows like an added
unique (§6.3), and runs as a staged change. A reference into `r` holds
a value of `r`'s key, so the operator retypes every field whose type is
`Key r` or `Key r.S`. Each such field, in any type, must be listed in
the `carry` clause by type, constructor and field; the engine rewrites
each listed field's value from the old key of the row it references to
the new key of the same row. A rekey that leaves any such field
unlisted is refused, and the report names each field. A field of `r`'s
own type typed `Key r` that is not in the new key must be dropped in
the same migration. The inverse is `rekey relation r key` with the old
key, carrying the same fields; it is exact while the old key's fields
remain. A rekey is relaxing when the new key has a field outside
the old key; the old key then stays enforced as a unique, held pending like
a dropped constraint (§6.4). A `drop field` of a field of an old key
held pending is refused while the old key is held. Migration 0051 is
such a rekey, since `sku` and `during` lie outside the old key `id`:
it is refused while any live bundle is pinned at version 50 or
earlier, and applies once none is.

### 4.11 Definitions

```adt fragment
-- 0010, from version 9
module auth
define fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
-- 0011 (§1.1) gives display_name its Apple arm
-- 0013, from version 12
module auth
redefine fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> "google:" ++ g.gid
  | Apple a     -> "apple:" ++ a.sub
-- 0014
module auth
drop fn display_name
```

`define fn` adds a schema function; `redefine fn` replaces its body,
and may change its signature; `drop fn` removes it. Every dependent
(schema functions, constraints, stored views, computed fields,
expression indexes) is rechecked at the new version (§3.2). Bundles
pinned earlier evaluate the definitions of their version (§7.1).

### 4.12 Constraints

`add constraint`, `add check`, `drop constraint` and `drop check`
change constraints without changing structure. §6 states their rules.

### 4.13 Retire a bundle

Retiring a bundle is a command, not an operator, and never appears in
a migration ([applications.md §4.3](applications.md#43-retirement)).
So is `supersede bundle B by B2`
([applications.md §4.4](applications.md#44-supersession)).

### 4.14 Scripts

A script is a write inside a migration, for transforms no operator
expresses:

```adt
-- 0013, from version 12 (§4.3), with
--   type CorpDomain = { suffix: Text }
--   relation corp_domains : CorpDomain key suffix
module auth
add constructor Identity.Corp { domain: Text }
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> scalar_len(c.domain) == 0
enable identities.Corp
deprecate identities.Google
script
  update i in identities.Google
  where exists d in corp_domains where ends_with(i.gid, d.suffix)
  set Corp { i with domain: i.gid }
```

The write after `script` is any write of
[writes.md](writes.md#1-change-sets): one change set, evaluated on the
state the operators before it in the migration produce, at the schema
they produce (§3.1). Its changes are checked against every constraint
of the final schema at the end of the migration, the migration's
statement boundary. A script changes no schema object.
Data fixes outside a migration are ad hoc writes or registered
transactions ([applications.md §7](applications.md#7-the-ad-hoc-role)).

A script changes data, not types, so translation across it is the
identity and a script never waits on pinned bundles. A script has no
inverse: `revert` of a migration containing one is refused. A restore
replays schema operators and skips scripts, since the restored data
is the state after them ([interop.md](interop.md#6-dump-and-restore)).

Rulings: evolution/add, evolution/remove, evolution/rename, evolution/split, evolution/merge, evolution/move-field, evolution/nest, evolution/split-routing/match-route, evolution/merge-tag/discriminator, evolution/hoist/default, evolution/remove-rows/restrict, evolution/remove-rows/merge-first, evolution/remove-rows/deprecate, evolution/log-ops, evolution/scripted, surface/ctor-set/growth/declare, constraint/scope/new-ctor/arm-in-add, derived/defaults/backfill/separate, spec/evolution/deprecated-value/same-path, spec/evolution/product-state/source-state, spec/evolution/new-ids/listed-order, spec/evolution/ctor-order/placed, spec/evolution/arm-target/scrutinee-set, spec/evolution/product-position/first-source, spec/late-relation-state/type-state, spec/record-add-ctor/allowed, spec/evolution/split-inverse/every-product, spec/evolution/split-self-ref/product-set, spec/evolution/field-action/field-type-and-alter

## 5. Inverses and revert

### 5.1 Inverse

Every operator except `script` has an inverse computed from the
operator and the schema before it (table in §4). The inverse names
objects by id: it is computed against the schema before the operator
and resolved to the objects' current names when it applies. An
operator and its inverse applied in sequence restore the prior state:
the schema, the write states and, for an exact operator, the data,
up to constructor ids. A constructor re-added by an inverse has a new
id and takes the position in the declaration order it had before its
removal.

### 5.2 Exact and lossy operators

An operator is exact when every database state at the version before
it maps to exactly one state after it and back. `drop field`, `move
field` down, and a split or merge whose routing is not invertible are lossy:
they state a loss, and the data they discard is not restored by their
inverse. Every other operator is exact. A lossy operator applies only
when no live bundle needs translation across the part it loses (§7.2).

### 5.3 Inverse of a migration

The inverse of a migration is the inverses of its operators in reverse
order. It is undefined if the migration contains a `script`.

### 5.4 Revert

```adt fragment
-- 0032, continuing §4.5
module auth
revert 0031
```

`revert n` is an operator whose meaning is the inverse of migration
`n` (§5.1). It appends a new migration; it does not remove `n` from
the log. Here the inverse is the renames of `0031` reversed, last
first, from `rename field Login.EmailPass.pw_hash to hash` to `rename
constructor Identity.Meta to Facebook`. The inverse is checked like
any migration (§3.2).

An inverse that needs values the log does not hold (the `add field`
inverse of a `drop field`) takes them from a `fill` clause of the
revert, one per such inverse, written `fill T.C.f = e` with the
field's current name. A revert's fill has the form and scope of an
`add field` fill (§4.9): `e` gives the value for every existing row
and may read the row being filled and nothing else.

```adt fragment
-- 0016, from a version 15 at which 0015 dropped Identity.EmailPass.hash
module auth
revert 0015
  fill Identity.EmailPass.hash = bytes("")
```

A revert is refused when the inverse of migration `n` is undefined,
when an inverse that needs a fill has none, or while a dependant of
`n` is applied. A later migration depends on `n` when one of its
operators names an object id that `n` created: reverting `0011` at
version 19 of §4.3 is refused, since `0012` and `0019` enable and
deprecate the `Apple` it added.

Rulings: evolution/smo, evolution/revert, evolution/merge-tag/discriminator, evolution/hoist/default, spec/evolution/enable-inverse/prior-state, spec/evolution/revert-identity/by-id, spec/evolution/product-position/first-source, spec/evolution/revert-fill/per-field, spec/evolution/field-action/field-type-and-alter

## 6. Constraints and grants under evolution

### 6.1 Constraint operators

`add constraint name = query`, `add check name on r.C (e)`, `drop
constraint name` and `drop check name` are operators. The forms of
constraints are in [constraints.md](constraints.md).

### 6.2 Carrying

Each structural operator rewrites the constraints it touches:

| Operator | Effect on constraints |
| --- | --- |
| rename | names are rewritten |
| split | a check or scope on the old constructor moves to the set of new ones: `check sub_present on identities.OAuth (...)` becomes `check sub_present on identities.(Google \| Apple) (...)` |
| merge | per-constructor checks become one check matching on the discriminator; per-constructor uniques become uniques including the discriminator |
| nest, unnest | scopes re-scope by leaf constructor, and paths change |
| move field | field references follow the field |
| add field, alter field | unchanged |
| add constructor | named sets it joins carry their constraints to it; match-scoped constraints need its arm (§4.2) |
| remove | arms of the removed constructor are dropped; its constructor relation reads as empty |
| rekey | key comparisons follow the carried fields |

An operator that leaves some constraint without an image is refused,
and the refusal names the constraint. Examples: a merge without a
discriminator of constructors with separate uniques; a remove of a
constructor a constraint requires rows of; a split whose routing a
constraint over the old constructor cannot follow. The user drops or
rewrites the constraint first, in the same or an earlier migration.

### 6.3 Adding a constraint

An added constraint, check or unique is validated against the state
the migration produces (§3.1). The migration is refused while any row violates it; the report
is the constraint's violation rows, as at a statement boundary. An
added constraint must be admitted
([constraints.md](constraints.md#5-admission)). Validation that runs
as a staged job and finds a violation fails the change (§3.4).

Bundles pinned before the constraint are not refused by it. A write
from such a bundle, translated to the current version, that violates
the constraint fails at call time as an ordinary constraint violation.

### 6.4 Dropping a constraint

A `drop constraint`, `drop check` or `drop unique`, or a relaxing
`rekey` (§4.10), applies and produces its version, but the dropped
constraint, check, unique or old key stays enforced while any live
bundle is pinned to a version that has it; until then the drop is
pending. Every reader sees only data that satisfies every constraint
of the version it reads. The pending drops and the bundles holding
each are the system relation `sys::pending_drops`
([catalog.md](catalog.md#1-system-relations)), and every migration
report names them. A drop takes effect when its last holding bundle is
released (§8).

Until its drop takes effect, a pending-dropped constraint is a schema
object for carrying (§6.2) and recheck (§3.2): later structural
operators rewrite it, and one that leaves it without an image is
refused, naming it.

### 6.5 Grants

Relation and constructor-set grants (`read`, `insert`, `update`,
`delete`, `retag`) and `grant call fn` are operators in the log: they
are hashed, printed and restored by replay. Roles, passwords,
capability grants and bundle call grants are deployment state, outside
the log ([security.md](security.md#3-grants)), as are grants on
`catalog::` and `sys::` relations, which are ad hoc statements and
never log operators. A grant with no field list covers the fields that
exist when it is granted, and a field added later to a constructor it
covers needs a new grant. A grant on a whole relation variable with no
field list also covers every field of each constructor the relation
variable gains.

Structural operators carry log grants; a field carried whole by a
split's routing keeps its grants under its new name. A merge that
cannot carry them is refused until it states the merged constructor's
grants, which `grant` operators in the same migration do
([security.md §3.6](security.md#36-grants-across-schema-changes)).
`move field` and `rekey` carry grants with the field. After the
operators of a migration, the engine rechecks every live bundle
against its owner's grants; a bundle whose owner no longer holds a
grant it uses refuses the migration.

Rulings: constraint/evolve/carry, constraint/evolve/icmo, constraint/evolve/unmapped/refuse, constraint/evolve/policy/check, constraint/evolve/drop-old/readers-first, constraint/scope/new-ctor/arm-in-add, spec/evolution/migration-statement/one-statement

## 7. Version translation

### 7.1 Serving a pinned version

A bundle pinned to version `v` keeps running when the current version
is later than `v`. Each of its queries and transactions means what it
meant at `v`, read over the current data mapped back to `v` through
the inverses of the operators between `v` and the current version, and
its writes are mapped forward through the operators. Results are
encoded at `v`: constructor names and wire tags are those of `v`.

Definitions under pinning:

- A pinned bundle evaluates the schema functions, aggregates and
  computed fields of its version `v`.
- Indexes and constraints always use the current definitions.
- A stored view whose definition is unchanged since `v` is read from
  its stored rows. One redefined since `v` is evaluated with its body
  at `v`, over the translated data, and its stored rows are not read.
- Names of the bundle's own functions resolve at `v`: a schema
  definition added later under the same name is allowed and does not
  change what the bundle calls.
- An expression index never changes a pinned bundle's results: its
  queries evaluate their expressions with the code of `v`
  ([interop.md](interop.md#95-function-identity)).
- A host function has one code: a `redefine fn` that changes a host
  function's code identity is refused while a live bundle pins a
  version with a different code identity for it.

| Operator after `v` | A query at `v` sees |
| --- | --- |
| rename (of any object) | the old name, same rows |
| split | rows of the new constructors mapped back through the inverse merge |
| merge | rows of the merged constructor split back by the discriminator |
| nest, unnest | the old shape |
| add field | rows without the field; its writes take the field as §7.4 states |
| drop field | refused at apply if it reads the field (§7.2); otherwise unchanged |
| move field up | the field on its old constructor; its writes of other constructors take the field as §7.4 states |
| move field down | the header field on the constructors that keep it; refused at apply if it reads the field of the others |
| rekey | references with the old key's values, mapped through the carried fields |
| add set, drop set, set membership | each named set with its membership at `v` |
| add constructor | the type without it; readers-first ensures it has no rows while the bundle reads the whole relation variable or follows a reference whose current type admits it, and, for a sum used as a field type, that no field the bundle reads holds a value of it (§8.1); a named or listed set it joins is read with its membership at `v` |
| enable, disable, deprecate | no change to reads; writes are checked against the current state |
| remove | the constructor, with no rows |
| define, redefine, drop fn, aggregate | the definitions of `v` |
| add, drop, redefine computed | the computed fields of `v` |
| install, uninstall | the definitions of `v`, under the host function rule above |
| adopt tzdb | the current zone rules: every bundle converts with the release in force now; stored zoned values keep their civil time and zone |
| add constraint | no change; its writes are checked against it (§6.3) |
| drop constraint, drop check, drop unique, relaxing rekey | the dropped constraint, check, unique or old key, still enforced (§6.4) |
| grant, revoke | the current grants of the bundle's owner |
| unique, index, default, store | no change to results |
| script | no change: translation is the identity |

### 7.2 Translation is decided at apply

When a migration applies, the engine translates every query and
transaction of every live bundle to the new version. If some query has
no translation, the migration is refused, and the report names the
bundle, the query and the operator. A query is translatable across an
operator when its writes have an image on every state they can produce
([semantics.md §9.6](semantics.md#96-version-translation)). That
criterion is decided by these forms; a query has no translation when:

- it writes into a constructor that the migration removes, disables
  or deprecates;
- it reads a field that a lossy operator drops for the rows it reads;
- it reads or writes a constructor that a split with non-invertible
  routing replaces;
- its translation across the operator does not type check.

A query reads field `f` when `f` occurs in a field access or a
pattern, or when a whole row having `f` is returned, compared or
passed to a function. Copying a row through `with` into a write does
not read its fields.

A live bundle is never left with a query that fails at call time
because of a schema change. A translated write that violates a
constraint added after `v` fails as a constraint violation (§6.3).

### 7.3 Expired bundles

A bundle whose lease has expired does not hold back migrations: §7.2
and §8 ignore it. Binds and calls on it, and renewals of its lease,
are refused with `Rejected { reason: BundleExpired {..} }` (§9). It is live
again only once registered again
([applications.md §4.1](applications.md#41-registration)), and that
registration translates every one of its queries and transactions to
the current version. The registration is refused with `BundleExpired`
when:

- any query or transaction has no translation;
- the bundle reads the whole relation variable of a constructor added
  after `v` and since enabled, or follows a reference whose current
  type admits such a constructor (§8.1), unless it declares `hide new
  constructors`;
- the bundle reads a field whose type reaches a sum that has gained,
  after `v`, a constructor since enabled (§8.1), whether or not it
  declares `hide new constructors`;
- a constraint, check, unique or key of version `v` has been dropped
  since and the drop has taken effect, so the bundle could read data
  its version excludes.

Otherwise the registration starts a new lease, and from then on the
bundle holds back later migrations as any live bundle does
([applications.md §4.2](applications.md#42-leases)).

### 7.4 Fields a translated write lacks

A write translated from version `v` states only the fields of `v`. A
field it lacks, added or moved after `v`, takes its value in this
order:

1. when the write updates or retags an existing row whose constructor
   is the constructor of the new value, the field's value in that row;
2. otherwise, and for every insert, the relation's current default for
   the field in that constructor
   ([derived.md §4](derived.md#4-defaults));
3. otherwise, the fill of the operator that added or moved the field.

```adt fragment
-- 0033, from version 32
module auth
add field Identity.EmailPass.verified: Bool fill false
add default identities.EmailPass.verified = true
-- a bundle pinned at 32 inserts EmailPass { ... }: verified is true
-- it retags a Google row to EmailPass { ... }: verified is true
-- it updates an EmailPass row: verified keeps the row's value
```

Rulings: surface/version-serving/translate, surface/version-serving/untranslatable/translate-at-apply, engine/pinned-version, surface/lease-policy/translate-stale, spec/evolution/translatable/semantic-via-triggers, spec/evolution/translated-fields/same-ctor

## 8. Readers first

A writer never emits a value that a live reader cannot type. Readers
are live bundles, their feeds and registered change consumers
([applications.md §3](applications.md#3-bundles)), each at its pinned
version. A bundle is released when it is retired, superseded or
expired; released bundles are not readers. A migration refused for a
reader, including an `enable` refused by readers-first, does not wait:
it applies when presented again after the readers named in its report
are released (§3.2). A revoke refused for a live bundle is handled the
same way ([security.md](security.md#3-grants)). A pending drop (§6.4)
takes effect, without being presented again, when the last bundle
holding it is released.

### 8.1 Enabling a constructor

`enable r.C` is refused while any reader is pinned at a version where
`C` does not exist and either reads the whole relation variable `r`
(one of its queries ranges over `r`, or one of its feeds names `r`
whole) or
follows a reference whose current type admits `C`: `Key r`, or `Key
r.S` where `S`'s current membership contains `C`. Dereferencing such a
reference could reach a `C` row. Ranging over a named or listed
constructor set of `r` does not block, since translation gives such a
set its membership at the reader's version, which lacks `C`.

`enable T.C`, for a sum `T` used as a field type, is refused while any
reader is pinned at a version where `C` does not exist and reads a
field whose type reaches `T`: the field's type is `T`, or a type whose
fields, constructors or type arguments reach `T` (`Option T`, a record
holding a `T`). Reading a whole row reads all its fields. A reader that
reads no such field does not block. `hide new constructors` does not
exempt a reader from this rule, since a value inside a row it can see
is never hidden.

A bundle whose header declares `hide new constructors` never blocks
`enable r.C`. Its reads of a whole relation variable, and references whose
type admits a constructor its version lacks, are translated to the
constructors of its pinned version, so rows of newer constructors are
invisible to it. Following a reference to a hidden row faults
`NoSuchRow`, so dereferencing and every path through a reference are
partial in such a bundle. Rows of constructors its version knows are
never hidden, including rows that reference hidden ones. A write of
such a bundle that conflicts with a hidden row still fails by key.

The refusal of an `enable` names each blocking bundle and states that
the bundle stops blocking only when it is released (§8): expiry needs
every client to stop calling it, and `sys::sessions` lists the
sessions still bound to it. For `BlockingNoticePeriod` after the
refusal the engine sends each session bound to a blocking bundle the
notice `BlockingMigration { bundle, version }` (§3.2), and an operator
may send `UpgradeRequired { bundle }`
([catalog.md](catalog.md#45-notices)).

The order for adding a constructor is therefore:

```adt
-- 0011 of §1.1: Apple declared, write-disabled
```

```adt
-- bundles/web.adb: registered with its Apple arm
bundle web-2.5 pins 11 (c7d2...) edition "2026-10"
import auth
query logins_of (u: Key users) -> Set Text =
  from i in identities where i.user == u
  select match i
    | EmailPass e -> e.email
    | Google g    -> g.gid
    | Apple a     -> a.sub
```

```adt fragment
-- 0012: applies once no reader pinned at 10 or earlier reads identities whole
module auth
enable identities.Apple
```

A reader pinned at `11` names `Apple` in its matches; one pinned at
`10` that reads `identities` meets no `Apple` row while it is live,
because none can be written until `0012`. A bundle pinned at `10`
that reads only `identities.OAuth` reads it as `Google` alone, before
and after `0012`.

### 8.2 Removing a constructor

Writers stop before readers lose a constructor: `deprecate` is refused
while a live bundle can write into the constructor relation (§4.3),
and `remove` is refused unless the constructor was deprecated by an
earlier migration (§4.4). Between the two, readers keep reading its
rows and its rows may be moved out.

### 8.3 Dropping a constraint

A drop stays pending while readers pinned to a version with the
constraint are live (§6.4).

### 8.4 Embedded databases

An embedded database applies these rules with the bundles registered
on the file as readers. Opening the file records the shipped bundles
and supersedes the bundles of earlier opens before it migrates
([applications.md §8](applications.md#8-embedded-databases)), so they
hold back nothing. The supersession commits durably on its own, before
the first migration; a later failure of the open does not undo it, and
only registering a superseded bundle again makes it live. A shipped
bundle recorded by the open is not a reader until the open registers
it, after the migrations.

Rulings: evolution/old-readers/readers-first, surface/registration/order/pinned, evolution/remove-rows/deprecate, constraint/evolve/drop-old/readers-first

## 9. Replication of schema changes

A replica serves consistent safe snapshots
([replication.md §1.2](replication.md#12-reads-on-a-replica)):
each snapshot holds the schema version and the data of one point of
the primary's history. A replica therefore never shows a row of a
constructor before the constructor's declaration.

A replica serves bundles bound to it at their pinned versions. The
primary is the only place leases are written and readers-first is
decided; how leases behave at a replica is in
[applications.md §4.2](applications.md#42-leases).

Rulings: repl/schema/in-log, repl/schema/forward-leases, spec/evolution/replica-schema/safe-snapshot

## 10. Engine releases

The catalog's system sums (sessions, plans, statistics, settings,
waits, and others; [catalog.md](catalog.md#1-system-relations)) are
written by the engine. Their schema has its own operator log, shipped
with each engine release: the catalog log. Its position and hash are
the catalog version. The catalog log is separate from the database's
operator log; nothing in it changes a schema version.

A release that changes a catalog sum ships the change as catalog
operators. They apply only when the operator runs `finalize upgrade`
([catalog.md](catalog.md#122-upgrades-and-downgrade)), and are never
appended at open; until then the database stays
at its catalog version and the release produces only values of it. At
finalize they follow §4 to §8 as user operators do. When readers-first
or another check of §3.2 refuses a catalog operator, `finalize
upgrade` is refused with a report naming the blockers; the upgrade
stays unfinalized and a downgrade stays possible. A constructor a
release adds to a catalog sum is declared, and the engine does not
produce a value of it until it is enabled. The engine enables it
itself once no live bundle pinned at an earlier catalog version reads
its whole relation; until then the behaviour that would produce it
stays off. Such an enable is deployment state, not a catalog operator,
and does not change the catalog version.

A bundle that reads catalog relations pins a catalog version in its
header, beside its schema version:

```adt fragment
bundle monitor-1.3 pins 9 (5d0e...) catalog 4 (e07b...) edition "2026-10"
query call_states () -> Set ({ call: Key sys::calls, state: Text }) =
  from c in sys::calls
  select { call: key c,
           state: match c.state
                    | Queued q  -> "queued"
                    | Running   -> "running"
                    | Waiting w -> "waiting" }
```

It is served at its catalog version through translation like any
pinned bundle. Registration and binding check the catalog version's
hash as they check the schema version's. A bundle without a `catalog`
clause may not read catalog relations.

Catalog sums evolve only through the catalog log. Spec editions
version only `Error`, `Notice` and the prelude sums
([README.md](README.md#editions)).

Rulings: ops/catalog-release/catalog-smo

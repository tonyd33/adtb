# Derived data

This part defines data computed from other data: views, stored views
and their maintenance, defaults and generated values, computed fields
and expression indexes, and reads of past states. It also states what
the engine does not derive (no rules, no triggers) and gives the
patterns by which valid time and event sourcing are expressed without
further mechanism.

Examples use the schema of [applications.md](applications.md) at
version `12`, extended locally; migration examples are one history
starting from a version `29` at which the invoices below exist, unless
a section says otherwise:

```
module billing
type Invoice { id: Key invoices, user: Key auth::users, amount: Decimal 2 }
  = Open
  | Paid { at: Instant }
  | Void { reason: Text }
relation invoices : Invoice
  key id generate sequence
  index (user) in Open
```

## 1. Views

A view is a schema function with no parameters. There is no separate
view declaration.

```
-- 0030, from version 29
module billing
define fn open_by_user () -> Set ({ user: Key auth::users, open: I64 }) =
  from u in auth::users
  select { user: key u, open: count(i for i in invoices.Open where i.user == key u) }
```

A view is read by calling it, `open_by_user()`, anywhere a set
expression is allowed, and denotes the value of its body at the
reader's snapshot and schema version: a bundle pinned to an earlier
version evaluates the definition of its version
([evolution.md §7.1](evolution.md#71-serving-a-pinned-version)). It is
a schema object, created,
redefined and dropped by operators and rechecked with the schema
([evolution.md §4.11](evolution.md#411-definitions)). A view whose
result is a sum has constructors like any query result.

Rulings: lang/nullary-fn

## 2. Stored views

### 2.1 Storing a view

```
-- 0031
module billing
store open_by_user
  key user
  index (open)
```

`store f` is an operator in the log. It applies only to a schema
function with no parameters; `store` of a function with parameters is
refused. `unstore f` is its inverse.

Storing a view does not change what it denotes. Every read of `f()`
returns the value of `f`'s body at the reader's snapshot, whether `f`
is stored or not. Storing is a physical choice: the engine keeps the
result as a relation and reads it instead of evaluating the body. No
statement refreshes a stored view and no reader can observe one out of
date.

A stored view is read only. A write naming it is a type error.

### 2.2 Admission

A view is stored only if its result has an index-bounded incremental
maintenance plan: for each base relation it reads, the change in the
view caused by a change set on that relation is computable by delta
queries whose every join from the delta probes an index. This is the
rule and the compiler that admit constraints
([constraints.md](constraints.md#5-admission)). A view with no such
plan is refused, and the refusal names each missing index or each
operator with no incremental form:

```
-- 0032
module billing
define fn paid_by_user () -> Set ({ user: Key auth::users, paid: I64 }) =
  from u in auth::users
  select { user: key u, paid: count(i for i in invoices.Paid where i.user == key u) }
store paid_by_user
-- refused: paid_by_user: no index (user) in invoices.Paid for the delta of invoices
```

A stored view's `min` or `max` is admitted only when an index on the
aggregated expression, prefixed by the group's key, exists on the
aggregated relation.

### 2.3 Freshness

A stored view is maintained inside every transaction that writes a
relation it reads. The view's change is computed from the
transaction's change set and becomes part of the same change set, so
the view and its base commit together. Every snapshot sees a stored
view consistent with the base relations at that snapshot; a
transaction that reads a stored view and its base is serializable as
if the view were evaluated.

A stored view is maintained by delta queries over the change set that
probe the base relations through their indexes at the transaction's
snapshot. The engine keeps no maintenance state other than the stored
relation itself and, for each stored row, the number of derivations
that produce it; a row leaves the view when its last derivation goes.

### 2.4 Concurrent maintenance

A change to a `count` or `sum` in a stored view is an increment that
commutes with other increments and records no read. Two transactions
whose writes only change the same aggregate row of a stored view do
not conflict through the view: two concurrent inserts of open invoices
for one user both commit. A transaction that reads the aggregate row
conflicts with concurrent writers to it by the ordinary rules
([transactions.md](transactions.md#1-serializability)).

A constraint that reads a stored aggregate row makes the increment a
read: a transaction whose change set changes an aggregate row that some
constraint reads records a read of that row, so two writers that
together violate the constraint conflict
([constraints.md](constraints.md#4-when-constraints-hold)).

### 2.5 What a stored view carries

A stored view is a relation and carries what a relation carries, with
one exception:

| Clause | Meaning on a stored view |
| --- | --- |
| `key f` | a key on the result, checked as a constraint through the view's change; when the key is implied by the body (a projection of a base key), the check is omitted |
| `index (f) in S`, `interval index` | indexes over constructor sets of the result, maintained with it |
| constraints | a constraint may read the stored view; it reads the stored rows and their change, as for base relations |
| `Key f` | not allowed: no reference type names a stored view |

```
constraint over_limit = from r in open_by_user() where r.open > 5 select r
```

`add index (open) in open_by_user` and `drop index` change a stored
view's indexes after it is stored.

### 2.6 Schema changes

A schema change that touches a stored view's inputs or result type
rechecks it like any schema function. Then:

- where the operator maps rows (a rename, a split or merge the view's
  body commutes with, a field move), the stored rows are mapped as the
  operator maps base rows;
- otherwise the stored relation is rebuilt as a staged job
  ([evolution.md §3.4](evolution.md#34-staged-changes)), and until the
  rebuild completes, reads of the view evaluate its body.

A `redefine fn` of a stored view rebuilds it. The redefinition must be
admitted (§2.2) or the migration is refused.

Rulings: derived/materialize/storage-choice, derived/admit/bounded, derived/freshness/in-txn, derived/freshness/hotspot/blind-delta, derived/ivm/delta-rules, derived/ivm/state/base-indexes, derived/mv-storage/heaps, derived/mv-integrity/keyed, derived/mv-integrity/indexed, derived/mv-integrity/constrained, derived/mv-evolve/translate, derived/mv-evolve/rebuild

## 3. No rules or triggers

No declaration makes a write cause another write. There are no rules,
no triggers and no procedures run on change. The only writes the
engine derives are delete actions on references
([writes.md](writes.md#7-delete-actions)); stored view maintenance
(§2.3) changes only stored views, whose content is a function of base
state.

State that must be written as a consequence of a change (an audit row,
a denormalised copy that is not a function of current state) is
written by the registered transaction that makes the change, in the
same change set:

```
-- 0033, from version 32
module billing
add type Payment = { id: Key payments, invoice: Key invoices, at: Instant }
add relation payments : Payment key id generate sequence
```

```
-- bundles/billing.adb
bundle billing-1.0 pins 33 (8d1f...) edition "2026-10"
import billing
txn pay (k: Key invoices, now: Instant) -> Key payments =
  transact
    update i in invoices.Open where key i == k set Paid { i with at: now }
    let p = insert payments { invoice: k, at: now }
    return key p
```

Rulings: derived/rules/none

## 4. Defaults

### 4.1 Defaults belong to relations

A default is a clause of a relation variable, not of a type. It
applies when a row enters that relation variable through a value that
omits the field:

```
relation sessions : Session
  key id generate uuidv7
  default Revoked.at = generate clock
type Counter = { name: Text, n: I64 }
relation counters : Counter
  key name
  default n = 0
```

| Form | Applies to |
| --- | --- |
| `default f = e` | field `f` of every constructor of the relation that has it |
| `default C.f = e` | field `f` of constructor relation `C` |
| `key f generate g` | the key field `f`, from generator `g` (`sequence`, `uuidv7`) |
| `default f = generate clock` | an `Instant` field, from the clock generator ([model.md](model.md#73-generated-keys)) |

The default expression `e` is a literal, a constructor of literals, or
`generate g`; it reads no relation and no other field. The values the
generators give are defined in [model.md](model.md#73-generated-keys):
`sequence` gives distinct values, and an explicitly given key value
advances it past that value; `uuidv7` gives distinct values and
nothing more; `clock` gives the wall-clock time at which the statement
applies, one value for every row of the statement.

### 4.2 Where a default applies

A default or generator fills an omitted field of a constructor value
written directly as:

- the value of an `insert` into the relation variable or one of its
  constructor relations, or the result of the `select` (or of an arm
  of a `match` that is that result) of the source query of `insert r
  from (...)`;
- the new value of a row in `update ... set` or in an arm of `set
  match`, when it retags the row into the constructor the default
  names;
- the target value of an `on delete retag C { ... }` action that
  retags a row of the relation.

A value built elsewhere (returned by a function, bound by `let`,
computed in a query that is not an insert's source) is a value of its
type and must give every field.
A field with no default that is omitted is a type error.

With the sessions above, deleting an identity retags its active
sessions by the action `on delete retag Revoked {}`, and
`Revoked.at` is filled by `generate clock`. The same holds for an
explicit retag:

```
update a in sessions.Active where a.who == k set Revoked {}
```

### 4.3 Defaults and new fields

A default governs inserts made after it exists. It never fills rows
that exist when it is added. Adding a field to a constructor states
its own fill for the existing rows, and may be followed by a default
for later inserts:

```
-- 0033
module auth
add field Identity.EmailPass.verified: Bool fill false
add default identities.EmailPass.verified = false
```

`add default r.f = e`, `add default r.C.f = e` and `drop default r.f`
are operators ([evolution.md §4](evolution.md#4-operators)).

Rulings: derived/defaults/on-relation, derived/defaults/backfill/separate

## 5. Computed fields

### 5.1 Virtual fields

A relation variable may declare a computed field: a field defined by
an expression over the row, evaluated on read and never stored.

```
type Shape { id: Key shapes } = Circle { r: F64 } | Rect { w: F64, h: F64 }
relation shapes : Shape
  key id generate sequence
  computed area: F64 = match self
    | Circle c -> 3.14159 * c.r * c.r
    | Rect r   -> r.w * r.h
```

`self` is the row. `s.area` reads the field of a row `s` of `shapes`
as a header field is read. For a binder over a constructor set, only
the arms of those constructors are evaluated. A computed field over a
sum is a match and must be exhaustive; adding a constructor states its
arm with an `arm` clause or redefines the field
([evolution.md §4.2](evolution.md#42-add-a-constructor)). A computed
field may not be given in a write, and may not be a key or the target
of a reference.

`add computed shapes.area: F64 = ...` and `drop computed shapes.area`
are operators; `redefine computed` replaces the expression.

### 5.2 Expression indexes

An index may range over an expression of the row, including a computed
field, instead of a stored field:

```
add index (area) in shapes
add index (lower(email)) in identities.EmailPass
```

The expression must be deterministic
([interop.md](interop.md#9-user-defined-functions)). The index is
maintained like any index; a query whose predicate or order matches the
expression may use it.

A unique may range over such expressions likewise; it is maintained as
a derived index and checked as any unique:

```
add unique (lower(email)) in identities.EmailPass
```

Rulings: derived/computed/virtual, derived/computed/expr-index

## 6. Reading the past

### 6.1 The retention window

A query may read any committed state within the retention window, a
database setting ([catalog.md](catalog.md#10-configuration)). The engine
keeps every row version that some snapshot within the window can see.
Versions older than both the oldest running snapshot and the window
are reclaimed as usual.

### 6.2 Naming a past state

A past state is named by a commit number. Commit numbers are `I64`
values that increase in the serial order of commits. The engine records
for each commit the wall time at which it committed, never decreasing
along commit numbers; the recorded map is the system relation
`sys::commit_times` ([catalog.md](catalog.md#1-system-relations)).

```
at commit 4120 { from i in invoices.Paid select i }
at instant t { from i in invoices.Paid select i }
from p in (at commit c { from i in invoices.Open select i }), q in invoices.Paid
where key p == key q
select { invoice: key q, paid: q.at }
```

`at commit c { q }` evaluates `q` at the state just after commit `c`.
`at instant t { q }` evaluates `q` at the latest commit whose recorded
time is at or before `t`. The block is refused at run time with
`Rejected { reason: HistoryUnavailable { oldest } }`, naming the oldest
retained position, when the commit is outside the retention window or
later than the reader's snapshot.

An `at` block is an expression of set type, usable anywhere a set is,
including beside reads of the current state in one query. It reads
only past state and writes nothing. A past read takes no SIREAD locks
and records no read dependency
([transactions.md](transactions.md#1-serializability)).

The block's body is typed at the schema version of its commit (§6.3);
its result type must denote the same type at the enclosing query's
version: base types, records and anonymous sums of such, and named
types whose constructors, ids and fields are equal at both versions.
Any other result type is a type error.

### 6.3 The schema of the past

A past read is typed against the schema version current at the commit
it reads, not against today's schema, and is not translated through
later operators. A registered query that reads the past names that
version as a bundle pins its own:

```
query paid_then (c: I64) -> Set Invoice =
  at commit c pins 29 (3b7e...) { from i in invoices.Paid select i }
```

The block is type checked at version `29` when the bundle is registered.
A call may name any retained commit whose schema version gives every
type the block reads (its relation variables, their constructors and
fields, and the functions it calls) the same definition as version
`29`; the block then reads that commit as if at `29`. A call whose
commit lies under a version where some such type differs is refused
with `Rejected { reason: WrongVersion { version } }` naming that
version. An ad hoc `at` block is
typed at the version of the commit it names when it runs.

### 6.4 Retention does not slow current reads

Retained versions must not change the cost of reading current state.
An implementation moves retained versions out of the current heaps
into a history store per constructor relation, so current scans and
index probes do not visit them; a past read reads both
([engine.md](engine.md#5-heaps-and-versions)).

Rulings: derived/time-travel/window, derived/time-travel/basis/both, derived/time-travel/past-schema/as-written, derived/time-travel/retention/history-heap

## 7. Valid time

The model has one notion of time, the commit order. Valid time, the
time a fact holds in the world, is a field of type `Span Instant` ([prelude.md](prelude.md)) with
an exclusion constraint against overlap:

```
type Price = { sku: Text, during: Span Instant, amount: Decimal 2 }
relation prices : Price
  key (sku, during)
  interval index (sku, during)
constraint no_overlap =
  from a in prices, b in prices
  where a.sku == b.sku and a.during != b.during and overlaps(a.during, b.during)
  select a
```

The interval index makes the constraint index-bounded. Changing the
price for part of a period is a transaction that deletes the covering
row and inserts the pieces.

A reference is checked by key only. That a referencing row's period
lies within its target's period is a constraint the user writes, and is
admitted when it is index-bounded:

```
type Department = { code: Text, during: Span Instant, name: Text }
relation departments : Department
  key (code, during)
  interval index (code, during)
type Assignment = { id: Key assignments, person: Key users, dept: Text, during: Span Instant }
relation assignments : Assignment key id generate sequence
constraint assignment_in_department =
  from a in assignments
  where not exists d in departments where d.code == a.dept and covers(d.during, a.during)
  select a
```

Rulings: derived/valid-time/pattern, derived/valid-time/temporal-fk/live

## 8. Event sourcing

The engine has no event store. An event log is an append-only relation
of an event sum, with state as stored views over it:

```
module orders
type OrderEvent { order: Text } = Placed { at: Instant }
                                | Shipped { at: Instant }
                                | Cancelled { reason: Text }
type Event = { seq: Key events, ev: OrderEvent }
relation events : Event key seq generate sequence
constraint events_append_only = from e in deleted(events) select e

fn order_counts () -> Set ({ order: Text, shipped: I64, cancelled: I64 }) =
  from e in events group e by e.ev.order into g
  select { order: g.key,
           shipped: count(x for x in g where match x.ev
                                               | Placed p    -> false
                                               | Shipped s   -> true
                                               | Cancelled c -> false),
           cancelled: count(x for x in g where match x.ev
                                                 | Placed p    -> false
                                                 | Shipped s   -> false
                                                 | Cancelled c -> true) }
store order_counts
```

`deleted(events)` reads the rows a change set deletes
([constraints.md](constraints.md#6-transition-constraints)). Every
projection over `OrderEvent` is an exhaustive match, so adding an event
constructor states its arm in each. Views are sets, not folds in event
order; a projection is built from aggregates that do not depend on
order.

Rulings: derived/event-sourcing/none

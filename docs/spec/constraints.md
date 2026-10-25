# Constraints

This part defines constraints: their one general form, a query that
must return nothing; the common kinds written as that form or as sugar
for it; where a constraint attaches in a sum; when constraints hold;
which constraints the schema admits; constraints that read the change
as well as the state; and what a schema change does to them. Keys,
uniques and references are declared on relation variables and types
([model.md](model.md#7-keys)); this part states how they are checked as
constraints.

Unless a section says otherwise, every example starts from version 12
of the evolution history: the running schema of [README.md](README.md)
after migration 0011 adds the constructor `Apple` and 0012 enables it,
so `Identity` is `EmailPass | Google | Apple` and `identities.OAuth` is
`Google | Apple`. Examples are independent of each other. They extend
that schema with:

```adt
type Entry = { id: Key entries, posted: Instant }
relation entries : Entry key id generate sequence
type Line = { id: Key lines, entry: Key entries on delete cascade, amount: I64 }
relation lines : Line
  key id generate sequence
  index (entry)

type Booking { id: Key bookings, resource: Text, during: Span Instant }
  = Room { seats: I64 }
  | Desk
relation bookings : Booking
  key id generate sequence
  interval index (resource, during) in (Room | Desk)

type Account { id: Key accounts, owner: Key users, opened: Instant }
  = Open
  | Closed { closed: Instant }
relation accounts : Account key id generate sequence

type Event = { id: Key events, at: Instant, body: Text }
relation events : Event key id generate sequence

type Place { id: Key places }
  = US   { zip: Text, state: Text }
  | Intl { country: Text, postcode: Text }
relation places : Place
  key id generate sequence
  index (zip) in US

type Membership = { id: Key memberships, user: Key users, org: Text }
relation memberships : Membership
  key id generate sequence
  unique (user, org)
type Assignment = { id: Key assignments, user: Key users, org: Text, task: Text }
relation assignments : Assignment key id generate sequence
```

and with an index on the owner of every identity, added by this
migration before the examples:

```adt
-- 0013, from version 12
module auth
add index (user) in identities
```

## 1. Constraints as denial queries

### 1.1 Form

```adt
constraint name = q
```

declares a constraint named `name`. `q` is a query in the query
language of [queries.md](queries.md). The constraint holds in a state
when `q` returns the empty set in that state. The rows `q` returns in a
state are that state's violations, and they are the violation report
(§7).

```adt
constraint has_identity =
  from u in users
  where not exists i in identities where i.user == key u
  select u
```

The `select` of `q` decides the report's shape; it may be the violating
rows, their keys, or any record built from them.

### 1.2 What a constraint query may read

`q` takes no parameters. It may read relation variables, constructor
relations, constructor sets and schema functions. It must not read:

- a file or other external source ([interop.md](interop.md#3-file-scans));
- a statistics or other volatile system relation
  ([catalog.md](catalog.md#1-system-relations));
- a function that is not declared deterministic
  ([interop.md](interop.md#9-user-defined-functions)).

Such a constraint is refused when declared. A constraint whose query
fails to type-check is refused.

### 1.3 Constraints are schema objects

A constraint is added by the operator `add constraint` and removed by
`drop constraint`, each an entry of the operator log
([evolution.md](evolution.md#4-operators)). Its name is unique
among the constraints of the schema. Every constraint, whatever the
form it was written in, is a denial query; the forms of §2 are notation
for it.

Rulings: constraint/form/empty, constraint/assertion, constraint/evolve/icmo

## 2. Common kinds

Each kind below is a denial query. Some have a shorter declaration;
the engine may check a kind with a dedicated mechanism, but its meaning
is always the denial query given here.

### 2.1 Keys, uniques and references

A key or `unique (f, ...) in S` of relation variable `r` holds when no
two rows of `r.S` agree on the fields. A reference field of type `Key
r.S` holds when every value it holds is the key of a row of `r.S`.
Both are declared where [model.md](model.md#7-keys) says and are checked
at the same boundaries as every other constraint (§4).

### 2.2 Row checks

```adt
check email_length on identities.EmailPass (scalar_len(email) <= 254)
```

is the constraint

```adt
constraint email_length =
  from x in identities.EmailPass where not (scalar_len(x.email) <= 254) select x
```

The expression of a check is a `Bool` expression over one row of the
scope. The row's fields are in scope by name and the row itself as
`self`. It may not read any relation. Scopes are in §3.

### 2.3 Domains

A predicate on a newtype, written `type N = new T where e` with `self`
naming the value, is a domain ([model.md](model.md#4-newtypes)):

```adt
type Email = new Text where matches(self, "^[^@]+@[^@]+$")
type Contact = { id: Key contacts, email: Email }
relation contacts : Contact key id generate sequence
```

Every value of the newtype written into a relation variable, at any
depth inside a row, must satisfy the predicate when the statement
ends. The predicate is checked only where values are stored;
constructing a value, `Email("a@b")`, never faults on its domain. It
is not part of the type that queries see: the type checker never assumes it, and a query
that needs it tests it.

### 2.4 Functional dependencies

```adt
constraint zip_state =
  from a in places.US, b in places.US
  where a.zip == b.zip and a.state != b.state
  select { zip: a.zip }
```

### 2.5 Exclusion

```adt
constraint no_double_booking =
  from a in bookings.(Room | Desk), b in bookings.(Room | Desk)
  where a.resource == b.resource and overlaps(a.during, b.during) and key a != key b
  select { a: key a, b: key b }
```

An exclusion under `==` on every compared field is a unique. `overlaps` and the type `Span T` are
in [prelude.md](prelude.md). An exclusion under `overlaps` on a range
field is admitted only with an
`interval index` over the scope whose leading fields are the equality
fields and whose last field is the range (§5.3).

### 2.6 Inclusion and at-least-one

```adt
constraint assigned_member =
  from a in assignments
  where not exists m in memberships where m.user == a.user and m.org == a.org
  select a
```

A reference is the inclusion whose target is a key. At-least-one
participation is inclusion from a parent into the rows that reference
it, as `has_identity` in §1.1. It holds at statement boundaries (§4),
so a parent and its first child are created in one statement.

### 2.7 Cardinality and aggregates

```adt
constraint identity_limit =
  from u in users
  where count(i for i in identities where i.user == key u) > 5
  select u

constraint balanced =
  from e in entries
  where sum(l.amount for l in lines where l.entry == key e) != 0
  select e
```

`count` and `sum` over an empty range give 0; `min` and `max` give an
`Option` ([queries.md](queries.md#10-aggregates)). Admission of
aggregates is in §5.2.

Rulings: constraint/row-check, constraint/domain, constraint/refine/write-only, constraint/fd, constraint/exclusion, constraint/inclusion, constraint/at-least-one, constraint/cardinality, constraint/assertion, constraint/overlap/range-index, constraint/overlap/equality-prefix

## 3. Scope

A row check and an exclusion attach to a constructor set of one
relation variable, as indexes do.

| Scope | Written | Meaning |
| --- | --- | --- |
| One constructor | `check c on r.C (e)` | `e` over the fields of `C` and the header |
| A constructor set | `check c on r.(C \| D) (e)`, `check c on r.S (e)` for a named set `S` | `e` over fields every member has; one check across the set |
| The sum, by match | `check c on r (match self \| C x -> e1 \| D y -> e2)` | one predicate per constructor |

A field used by a check over a constructor set must be a field of
every constructor in the set, with one type; otherwise the check is a
type error. An exclusion over a constructor set compares rows of all
its members with each other, so two bookings of one resource conflict
whether each is a `Room` or a `Desk`.

A match-scoped check is exhaustive like any match: every constructor
has an arm, `true` included.

```adt
check login_valid on identities (match self
  | EmailPass e -> scalar_len(e.email) <= 254
  | Google g    -> scalar_len(g.gid) > 0
  | Apple a     -> true)
```

An explicit constructor set never grows. A named set grows only as
each added constructor declares it joins the set. Adding a constructor
to a sum must give its arm of every match-scoped constraint on the
sum's relation variables, in the `add constructor` operator. In a
history starting at version 10, before `Apple` exists, `login_valid`
has arms for `EmailPass` and `Google` only, and `identities.OAuth` is
`Google`; migration 0011 adds `Apple` with its arm:

```adt
-- 0011
module auth
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
  arm login_valid: Apple a -> scalar_len(a.sub) > 0
  arm display_name: Apple a -> a.sub
```

The new arm is checked against the new constructor relation, which is
empty.

A check whose scope is the whole relation variable, written without a
constructor set, covers every constructor of the sum, including
constructors added later, as an index over the whole sum does.

Rulings: constraint/scope/ctor, constraint/scope/ctor-set, constraint/scope/match, constraint/scope/new-ctor/arm-in-add

## 4. When constraints hold

Every constraint holds at every statement boundary: in the state
before each statement and in the state after it. A statement is one
change set applied as one multiple assignment
([writes.md §1](writes.md#1-change-sets)): a `transact` block, a
registered transaction, one item of ad hoc text, or one call of an
interactive transaction ([transactions.md §2](transactions.md#2-statements)).
The items of a `transact` block are not statements: the block is one.

No state inside a statement is observable, so no constraint is checked
inside one. No constraint is deferred to commit and there is no
declaration that defers one. Rows that must exist together, including
reference cycles, are created in one statement:

```adt
transact
  let u = insert users { name: n }
  let i = insert identities.Google { user: key u, gid: g }
  return key i
```

A statement that would leave any constraint violated fails, and none
of its change set is applied.

Constraints hold after every call of an interactive transaction, so an
interactive transaction cannot pass through a violating state between
calls. A change that must be made in several steps, such as a user and
its first identity under `has_identity`, is made by one `transact`
block or registered transaction, called once
([transactions.md §5.2](transactions.md#52-meaning)).

Rulings: constraint/timing/multi-assign, model/deferred-check, write/unit/multi

## 5. Admission

### 5.1 The bound

A constraint is admitted only when the engine can check it with an
index-bounded incremental check: for every relation the query reads,
a check over the rows a change set inserts or deletes in that relation,
in which every other relation the query reaches is read only through
index probes on the fields that connect them. The cost of checking a
change set is then bounded by the size of the change set times the cost
of the probes, independent of the size of the database.

A constraint without such a check is refused when declared. The
refusal names, for each relation whose changes cannot be checked so,
the missing index or the operator that is outside the bound. Dropping
an index that an admitted constraint's check needs is refused, naming
the constraint.

```adt
constraint has_identity =
  from u in users
  where not exists i in identities where i.user == key u
  select u
-- admitted: for inserted users, index (user) on identities;
--           for identities deleted or moved to another user,
--           key of users, then index (user) on identities
```

### 5.2 Shapes admitted

Within the bound, a constraint may use:

- joins, filters and `exists`/`not exists` whose correlations are
  covered by indexes;
- the transition relations of §6;
- `count`, `sum`, `min` and `max` of a field over the rows that share a
  group key, where the group key is the leading fields of an index;
  the engine maintains each such aggregate per group from each change
  set, as an index is maintained. `min` and `max` also need an index on
  the aggregated field within the group, for deletes;
- comparisons and functions applied to these.

Recursion (`fix`) is outside the bound. Other shapes are admitted when
the bound above holds for them and refused otherwise.

### 5.3 Exclusion over ranges

An exclusion that compares a range field under `overlaps` is within the
bound when the scope has an `interval index` whose leading fields are
the fields the exclusion compares under `==`, followed by the range
field. The check probes the index with each written row's equality
fields and range.

Rulings: constraint/admit/bounded, constraint/enforce/delta, constraint/aggregates/maintained, constraint/overlap/range-index, constraint/overlap/equality-prefix

## 6. Transition constraints

### 6.1 Reading the change

Within a constraint, for each constructor relation, constructor set or
relation variable `R`:

- `inserted(R)` is the set of rows the change set inserts into `R`;
- `deleted(R)` is the set of rows the change set deletes from `R`, as
  they were in the pre-state;
- `prior(R)` is `R` in the pre-state.

Unqualified relation names read the state after the change. The change
set is the normalised one of [writes.md §1.2](writes.md#12-what-a-change-set-is):
an update appears as its old row in `deleted` and its new row in
`inserted`, with one key; an unchanged row appears in neither. A retag
from `C` to `D` appears in `deleted(r.C)` and `inserted(r.D)`.

```adt
constraint append_only = from e in deleted(events) select e

constraint no_reopen =
  from o in deleted(accounts.Closed), n in inserted(accounts.Open)
  where key o == key n
  select n

constraint opened_fixed =
  from o in deleted(accounts), n in inserted(accounts)
  where key o == key n and o.opened != n.opened
  select n
```

The last reads `opened` on the sum because it is a header field.

A binder over `inserted(R)`, `deleted(R)` or `prior(R)` ranges over
rows of `R` and is statically known as a binder over `R` is, so `key o`
applies to it ([semantics.md §3.2](semantics.md#32-rules)).

### 6.2 Checking

A transition constraint is admitted and checked as any other (§5). In
the state before the first statement that follows its addition, and
whenever it is evaluated against existing data (§9.2), the change is
empty: `inserted` and `deleted` are empty and `prior(R)` is `R`.

Rulings: derived/transition/change-visible, write/primitive/delta

## 7. Checking and violation reports

A statement's change set is checked before it is applied. For every
admitted constraint that reads a relation the change set touches, the
engine runs the constraint's incremental check against the change set
and the states before and after it. Constraints reading no touched
relation are not checked.

Because every constraint held before the statement, the violations
after it are exactly the rows the constraint query returns in the
state after it. When any constraint has a violation:

- the statement fails and none of its change set is applied;
- the error is `Violated` ([catalog.md](catalog.md#4-error-sums)) and
  carries one constraint: of the violated constraints, the one whose
  qualified name sorts first, with its report, the rows its query
  returns, sent
  as a result descriptor followed by rows ([wire.md](wire.md));
- the caller sees the report rows it may read and the others redacted
  ([security.md](security.md#3-grants)): a registered call by the
  rights its bundle grants, an ad hoc caller by its own grants.

A fault while evaluating a check fails the statement as a fault does
([queries.md](queries.md#15-faults)).

Within an interactive transaction, a failed call leaves the
transaction open, with the effects of its earlier calls; the reads the
failed call made remain part of the transaction for serializability.

Rulings: constraint/enforce/delta, constraint/form/empty

## 8. Concurrent transactions

Constraint checks are reads of the transaction that makes the change,
tracked as every read of a serializable transaction is
([transactions.md §1](transactions.md#1-serializability)). Concurrent
transactions therefore cannot jointly violate a constraint that each
would satisfy alone. If two transactions each delete one of a user's
two identities, both checks of `has_identity` pass on their own
snapshots, and at most one of the two commits; the other fails with a
serialization failure and may be retried.

A check that reads a stored view or a maintained aggregate records that
read like any other, even where writes maintaining the view record
none ([derived.md](derived.md)), so a constraint over a stored total
cannot be jointly violated either.

Rulings: constraint/concurrent/serial-checks, engine/write-skew/by-isolation, engine/serializable-only

## 9. Constraints under schema change

### 9.1 Structural operators carry constraints

Each structural operator rewrites the constraints it touches into the
new schema: a rename renames the names it mentions, a field move
moves the field reference, a split rescopes a constraint on the old
constructor to the constructors it is split into, and a merge rewrites
per-constructor constraints into ones over the merged constructor
using its discriminator.

In a history in which `Identity` is `EmailPass | Social { provider:
Provider, sub: Text }`, with `type Provider = Google | Facebook`, before
migration 0014:

```adt
-- 0014
module auth
split constructor Identity.Social into Google { gid: Text } | Facebook { fid: Text }
  by match
    | Social { provider: Google, sub }   -> Google { gid: sub }
    | Social { provider: Facebook, sub } -> Facebook { fid: sub }
  arm display_name: Google g -> g.gid
  arm display_name: Facebook f -> f.fid
-- check sub_present on identities.Social (scalar_len(sub) > 0)
--   becomes check on identities.Google (scalar_len(gid) > 0)
--   and check on identities.Facebook (scalar_len(fid) > 0)
```

A constraint whose drop is pending (§9.3) is still a schema object:
structural operators carry it and it is rechecked as any other. An
operator that has no image for some constraint, a pending one
included, is refused, naming each such constraint. The schema change
proceeds only after the constraint is rewritten or dropped by its own operator.

### 9.2 Adding a constraint

`add constraint` evaluates the constraint query against the existing
data. While it returns any row the operator is refused, and the
refusal carries the violation rows. The inverse of an admitted `add
constraint` is `drop constraint`.

Starting from version 12:

```adt
-- 0013
module auth
add constraint balanced = ...
-- refused: Violated balanced, its report holds the 3 violating entries
```

### 9.3 Dropping a constraint

A `drop constraint`, a `drop unique`, a `drop check` and a `rekey`
that relaxes a key each take effect only once no live bundle is pinned
to a version that has the constraint it removes, for a `rekey` the
stricter key
([applications.md](applications.md#6-what-a-schema-change-does-to-bundles)). Until then the
engine keeps checking it, so every reader sees only data that
satisfies every constraint of the version it reads. The report of the
migration that drops it names the drop as pending and each bundle
holding it back, and every pending drop with its blocking bundles is a
row of `sys::pending_drops`
([evolution.md §6.4](evolution.md#64-dropping-a-constraint),
[catalog.md §6.6](catalog.md#66-pending-drops-and-request-marks)), so a write
refused by a dropped constraint can be traced to the bundle that keeps
it alive. The inverse of a drop is the matching add, and of a relaxing
`rekey` the `rekey` back, validated against the data as in §9.2.

A constraint is always checked with the current definitions of the
schema functions it calls, whatever version the writing bundle is
pinned to.

Rulings: constraint/evolve/carry, constraint/evolve/icmo, constraint/evolve/unmapped/refuse, constraint/evolve/policy/check, constraint/evolve/drop-old/readers-first, constraint/scope/new-ctor/arm-in-add

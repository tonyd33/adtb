# Queries

This part states what a query means: the value it denotes for a
database state and its parameters, which queries are refused, and how
evaluation behaves when an operation faults. It covers relations and
constructor relations, comprehensions, records and result sums,
`match` and patterns, navigation, nested results, grouping and
aggregates, recursion, functions and views, parameters, ordering and
limits, and faults. The grammar of every form is in
[syntax.md](syntax.md#6-expressions); typing rules are in
[typing.md](typing.md); the kernel calculus each form desugars to is
in [semantics.md](semantics.md).

Examples use the running schema of the [index](README.md#conventions)
at version 12, after migration 0011
([evolution.md §1.1](evolution.md#11-migrations)) adds `Apple { sub:
Text }` joining `identities.OAuth`, with its `display_name` arm, and
`0012 enable identities.Apple`, extended for this part with:

```adt
type Post = { id: Key posts, author: Key users, created_at: Instant, title: Text }
type Phone = { id: Key phones, owner: Key users on delete cascade, seen: Instant }
type Follow = { src: Key users, dst: Key users }
type Employee = { id: Key employees, name: Text, manager: Option (Key employees) }
type Order = { id: Key orders, user: Key users, total: I64, qty: I64 }
type Reset = { id: Key resets, identity: Key identities.EmailPass on delete cascade }
type Raw = { id: Key raws, n: I64 }
relation posts : Post key id generate sequence
relation phones : Phone key id generate sequence
relation follows : Follow key (src, dst)
relation employees : Employee key id generate sequence
relation orders : Order key id generate sequence
relation resets : Reset key id generate sequence
relation raws : Raw key id generate sequence
```

## 1. Evaluation model

### 1.1 Queries are pure

A query is an expression. Its value is determined by its parameters and
one snapshot of the database: evaluating it has no effect, and two
evaluations against the same snapshot with the same parameters give
the same value. Every relation variable a query reads is read at that
one snapshot. Writes are change-set values applied by a transaction
([writes.md](writes.md#1-change-sets)); no query form changes the
database.

A query has no clock. The current time reaches a query only as a
parameter, or as a stored field an insert filled from a relation's
`generate clock` default ([derived.md](derived.md#4-defaults)).

### 1.2 Where queries run

A query runs in one of these forms:

| Form | Declared by | Checked | Rechecked on schema change |
| --- | --- | --- | --- |
| Registered query | `query` in a bundle | at registration | yes |
| Schema function or view | `fn` in a migration or a bundle | at definition | yes |
| Constraint or check | `constraint`, `check` | at admission | yes |
| Ad hoc statement | text sent by a principal holding the ad hoc grant | when run | no |

A registered query is a named, typed catalog object called by name
with values for its parameters ([applications.md](applications.md#1-registered-queries-and-transactions)).
Ad hoc statements are described in §16. There is no SQL surface.

### 1.3 Values and equality

Every value of a first-order type has the structural equality `==` of
[model.md](model.md#9-equality). Equal values are indistinguishable by
any query, so wherever this part says that equal values are merged,
which of them is kept cannot be observed.

Rulings: lang/family/comprehension, write/surface/txdata, types/now/param-only, lang/registered/named, lang/defs-home/schema, lang/defs-home/bundle, query/tooling/live, types/eq/leibniz, types/eq-types/all-first-order

## 2. Relations and constructor relations

### 2.1 Relation expressions

A relation variable named in an expression denotes the set of its rows
at the snapshot. A row is a value of the relation's type, its key
field included, so two rows of one relation variable are never equal.

Every relation is a sum. For a relation variable `r` of type `T` with
constructors `C1 ... Cn`:

| Expression | Denotes | Element type |
| --- | --- | --- |
| `r` | every row of `r` | `T` |
| `r.C` | the rows of `r` whose constructor is `C` | `T.C` |
| `r.(C \| D)` | the rows whose constructor is `C` or `D` | `T.(C \| D)` |
| `r.S`, `S` a named set of `r` | the rows whose constructor is a member of `S` | `T.(members of S)` |

Each row belongs to exactly one constructor relation, and `r` is the
disjoint union of `r.C1 ... r.Cn`. A constructor set's relation is the
disjoint union of its members' relations.

A record type is a one-constructor sum, so for `relation users : User`
the relation `users` and the constructor relation `users.User` hold the
same rows.

### 2.2 Row types

`T.C` is the row type of constructor `C`; `T.(C | D)` the row type of a
constructor set. A row type is a subtype of `T`
([typing.md](typing.md#3-subtyping)). A value of a row type has the
sum's header fields and the constructor's own fields; a value of a
constructor set's row type has the header fields and the fields every
member declares at one type. `key e` is the key of row `e`; for `e`
ranging over `r.S`, `key e` has type `Key r.S`, which widens to `Key
r`. `key x` is defined exactly when the relation of `x` is statically
known:

- `x` is a binder over a relation variable, a constructor set, a
  backlink, a key-preserving query (one whose `select` is the binder
  itself), or a view that returns rows of one relation;
- `x` is a binder over `inserted(R)`, `deleted(R)` or `prior(R)` in a
  transition constraint
  ([constraints.md](constraints.md#61-reading-the-change));
- `x` is the binder of a `match` arm on such a row;
- `x` is a row bound by a `let` from a write;
- `x` is a value of a type with a self-identifying `Key r` field.

Anywhere else `key x` is refused.

`Key r` is a distinct type for every relation variable `r`, whatever
the types of its key fields, and is represented by the key fields'
value. A key value is built only by `r.key(v)`, or `r.key(f: v, g: w)`
for a composite key, read back by `value(k)`, and compared with the key
of a row: `i.user ==
users.key(3)`, `key m == memberships.key(user: u, team: t)`. No
literal checks against a `Key` type, whatever the key's generator
([model.md](model.md#7-keys)).

### 2.3 Other set-valued expressions

Any expression of a `Set` type can be ranged over: a comprehension, a
call of a set-valued function or view, a backlink (§7.3), a group
(§9), a `fix` binder (§11), a `top` (§14.3), a nested field of a
result, or a `Set` parameter of a function.

When the element type of a set-valued expression `q` is a sum, `q.C`
and `q.(C | D)` select its elements by constructor exactly as for a
relation variable. This holds for sums a query builds (§4.3):

```adt
fn sign_ins () -> Set (Local { id: Key identities, email: Text }
                     | Remote { id: Key identities, provider: Text }) =
  from i in identities select match i
    | EmailPass e -> Local  { id: key e, email: e.email }
    | Google g    -> Remote { id: key g, provider: "google" }
    | Apple a     -> Remote { id: key a, provider: "apple" }

from l in sign_ins().Remote select l.provider
```

There is no separate narrowing operator on single values; a value's
constructor is examined by `match`.

Rulings: query/ctor-relations, model/one-ctor-sum, model/sets, lang/binder/ctor-row, model/sum-results, surface/ctor-set/named, surface/ctor-set/list

## 3. Comprehensions

### 3.1 Meaning

A comprehension

```adt
from x1 in e1, x2 in e2 where p select v
```

denotes the set of the values of `v` over every binding of `x1, x2`
for which `p` holds. Precisely, evaluation keeps a sequence of
bindings, starting from one empty binding, and applies the clauses in
the order written:

| Clause | Effect on each current binding |
| --- | --- |
| `from x in e` | evaluate `e` under the binding; replace the binding by one extension per element of the result, with `x` bound to it |
| `where p` | evaluate `p` under the binding; keep the binding only if `p` is `true` |
| `group e by k into g` | replace all bindings by one binding per group (§9) |
| `select v` | evaluate `v` under the binding; its value is an element of the result |

A comma in `from` is shorthand for a second `from`. Each clause sees
the binders introduced before it, together with parameters and the
binders of enclosing expressions, so a generator may depend on an
earlier one:

```adt
from u in users, i in u.<identities.user
select { name: u.name, id: key i }
```

`where` may be written several times; their conjunction is evaluated in
the order written (§15.2).

### 3.2 Results are sets

The value of a comprehension is a set: elements equal under `==` are
one element. Projection is where equal values arise:

```adt
from p in posts select p.author     -- each author once
```

A comprehension's value has a type `Set T`. There are no bags and no
lists. A query or function result may be of any type: `query n_users
() -> I64 = count(u for u in users)`.

### 3.3 Union

`a union b` is the set union of two sets whose element types join
([typing.md](typing.md#5-joins)). Equal elements from the two operands
are one element.

### 3.4 Emptiness tests

`exists x in e where p` is `true` when at least one binding of `x`
over `e` satisfies `p`, and `false` otherwise; with several generators
it ranges over their bindings as `from` does. `not exists ...` is its
negation. `exists x in e where p` has the same value as
`count(x for x in e where p) > 0`.

Rulings: lang/family/comprehension, lang/collection/sets, model/sets, model/dup-arms/dedupe, types/eq/leibniz

## 4. Records and result types

### 4.1 Anonymous records

A record expression `{ f1: e1, ..., fn: en }` built by a query has an
anonymous record type `{ f1: T1, ..., fn: Tn }`. Anonymous record types
are structural: two are the same type exactly when they have the same
field names with the same types. There is no width subtyping between
them: `{ id: Key users, name: Text }` is not a `{ id: Key users }`,
and a query whose declared result has fewer fields than the records it
builds is refused. The fields wanted are projected.

Stored types are nominal: a value of `User` and an anonymous record
with the same fields are different types.

### 4.2 Arm and operand types

When the arms of a `match`, or the operands of a `union`, have
different types, the result has their join: rows of different
constructor sets of one sum join at the union of the sets, `Key r.S`
and `Key r.T` join at `Key r.(S | T)`, and anonymous records join only
when their types are equal. Values of different named types do not
join, and an implementation must refuse them. Arms whose shapes differ
are kept apart by building constructors of a result sum (§4.3).

```adt
from i in identities select match i
  | Google g    -> g
  | Apple a     -> a
  | EmailPass e -> e          -- Set Identity
```

### 4.3 Result sums

A query result may be a sum: the arms of a `match` build constructors,
and the result is the disjoint union of the arms with each element
keeping its constructor. A result sum is either a declared type or a
sum anonymous to the query, written in the signature:

```adt
query login_key (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text }) =
  from i in identities.(EmailPass | Google) where i.user == u
  select match i
    | EmailPass e -> ByEmail { email: e.email }
    | Google g    -> ByGoogle { gid: g.gid }
```

The constructors of an anonymous result sum are structural: their
names and payload types are their identity within the query, and they
have no catalog identity. A result sum is matched, ranged over by
constructor (`q.C`) and checked for exhaustiveness exactly as a stored
sum is.

In a function, registered query or constraint, an anonymous
constructor expression is typed by the expected type that names it; a
constructor name with no expected type that declares it and no
declaration in scope is refused. A comprehension checked against `Set
T` checks its `select` against `T`, so the arms of `login_key` above
are checked against the signature's result sum
([typing.md](typing.md#22-the-algorithm)).

An ad hoc statement has no signature: the type of its result, an
anonymous result sum included, is inferred from the statement.

### 4.4 Constructor tags on the wire

A stored constructor is identified on the wire by its catalog
constructor id, which is never reused. A constructor of a sum anonymous
to a query is numbered by its position in the query's signature,
counting from 1; `ByEmail` above is 1 and `ByGoogle` 2. In an ad hoc
statement, whose result sum is inferred, the constructors are numbered
by their first appearance in the statement text, counting from 1.
Numbering follows the text; it does not change type equality, under
which two anonymous sums with the same constructors in another order
are one type ([model.md](model.md)). A changed
signature is a new query ([applications.md](applications.md#1-registered-queries-and-transactions)).
The encoding is in [wire.md](wire.md#43-records-and-sums).

Rulings: model/anon-results/anon-records, types/sub-records/exact, types/join/lub-exact-fields, model/sum-results, surface/query-scoped, surface/sum-encoding/stable-tag, lang/record-poly/nominal

## 5. Match

### 5.1 Meaning

```adt
match e
  | p1 -> v1
  | p2 if g2 -> v2
  ...
```

evaluates `e`, then tries the arms in the order written. An arm is
chosen when its pattern matches the value (§6) and its guard, if any,
evaluates to `true`; the match's value is the chosen arm's expression,
evaluated with the pattern's binders bound. A match accepted by the
checker always chooses an arm (§5.3).

`match` is an expression: it may appear in `select`, `where`, a group
key, an aggregate, a function body or another arm.

`if c then a else b` is `match c | True -> a | False -> b`.

### 5.2 Scrutinees

| Scrutinee type | Matched as |
| --- | --- |
| a row type of a relation (a binder over `r`, `r.S`, a backlink, ...) | the row, by its constructor |
| a sum type (a field of value-level sum type, a sum parameter, a result element) | the value, by its constructor |
| `Key r.S` | the row the key references, as for a row of `r.S` |
| a record type | the value, as a one-constructor sum |
| a scalar type (`Text`, integers, floats, `Decimal`, ...) | the value, by literal patterns |

A scrutinee of type `Key r.S` is matched by the constructor of the row
it references, dereferenced at the snapshot; `_` and a binder are not
allowed as its top-level pattern, in ad hoc text included. When the
scrutinee is a reference field of a row, the row exists, because
references are valid at every statement boundary
([constraints.md](constraints.md#21-keys-uniques-and-references)).
Any other key, such as a parameter or `r.key(v)`, may name no row of
`r.S`; the match then faults with `NoSuchRow` (§15), and it is partial
([typing.md](typing.md#101-total-and-partial-expressions)).

### 5.3 Exhaustiveness

A match over a sum must name every constructor of the scrutinee's type
in an unguarded arm, directly, through an or-pattern, or through a
named-set pattern (§6.4). The constructors to cover are those of the
scrutinee's static type: a match over a row of `identities.(Google |
Apple)` covers `Google` and `Apple` only. Guarded arms do not count
toward coverage (§6.6).

A match over a scalar ends with an unguarded arm whose pattern is `_`
or a binder.

Coverage of nested patterns is decided over every position the arms
discriminate (§6.3). A match that is not exhaustive is refused at
compile time as a type error: at registration for a registered query,
at definition for a schema function or constraint, at the recheck that
follows a schema change for every dependent, and when run for an ad
hoc statement. A match is never evaluated against a value no arm
covers.

### 5.4 Wildcards over sums

Outside the ad hoc role, `_` and a binder pattern never stand for the
constructors of a sum. At the scrutinee position of a match over a
sum, and at any nested position of sum type where some arm names a
constructor, every arm names constructors there. A sum-typed position
no arm discriminates may be `_`, a binder, `C _`, or a field left out
of a record pattern. So adding a constructor to a sum makes every match
that branches on that sum fail its recheck until the constructor is
handled, or until the constructor joins a named set the match already
covers.

In an ad hoc statement `_` and a binder may cover the remaining
constructors of a sum.

```adt
-- refused in a registered query; accepted ad hoc
match i | Google g -> g.gid | _ -> "other"
```

### 5.5 Binders

A constructor pattern `C x` binds `x` to the matched value at the
constructor's type:

- over a row of relation `r` of type `T`, `x` has the row type `T.C`:
  it has the key and header fields, `key x` is a `Key r.C`, and `x`
  may be passed or returned wherever a `T` is expected;
- over a value of a value-level sum `V`, `x` has type `V.C`, a subtype
  of `V`, with `C`'s fields and no key.

```adt
from i in identities select match i
  | EmailPass e -> { owner: e.user, login: e.email }
  | Google g    -> { owner: g.user, login: g.gid }
  | Apple a     -> { owner: a.user, login: a.sub }
```

### 5.6 A match over a relation is a union of arm queries

Let `Q` be a comprehension with no `group` and a generator `x in r`,
`r` of type `T` with constructors `C1 ... Cn`, in which `x` occurs
neither inside an aggregate nor inside the range of a `top`, and let
every `match x` in `Q` be exhaustive.
Then `Q` denotes the union over `k` of `Qk`, where `Qk` is `Q` with
the generator replaced by `x in r.Ck` and each `match x` replaced by
its arms that can match `Ck`, in their order. Each `Qk` reads only the
constructor relation `r.Ck`. The arms range over disjoint inputs; equal
outputs of different arms are one element of the result (§3.2).

```adt
from i in identities where i.user == u
select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub
```

is

```adt
(from e in identities.EmailPass where e.user == u select e.email)
union (from g in identities.Google where g.user == u select g.gid)
union (from a in identities.Apple where a.user == u select a.sub)
```

An implementation evaluates such a match as one probe of the union
index that routes each entry to its arm by constructor
([engine.md](engine.md#61-constructor-set-indexes)); the value is the union above.

### 5.7 No null

There is no NULL value and no implicit arm for a missing value.
Optionality is a constructor: a field that may be absent has a sum
type such as the prelude's `Option T = None | Some { value: T }`, and
is matched like any sum.

Rulings: query/construct, query/compile-time, query/union-all, query/native, query/optional-sum/null-ctor, lang/wildcard/scalars-only, lang/wildcard/verbosity/adhoc-wildcard, lang/binder/ctor-row, evolution/breaks/recompile, model/user-params

## 6. Patterns

### 6.1 Forms

| Pattern | Matches | Binds |
| --- | --- | --- |
| `_` | any value (scalar positions, and §5.4) | nothing |
| `x` | any value (scalar positions, and §5.4) | `x` to the value |
| `C` | a nullary constructor `C` | nothing |
| `C _` | any value with constructor `C` | nothing |
| `C x` | any value with constructor `C` | `x` to the value at type `T.C` |
| `C { f: p, g }` | a value with constructor `C` whose field `f` matches `p` | the binders of `p`, and `g` to field `g` |
| `S x`, `S` a named set | a row whose constructor is a member of `S` | `x` at the set's row type |
| literal | a scalar equal (`==`) to the literal | nothing |
| `p1 \| p2` | a value either alternative matches | the names every alternative binds |

`true` and `false` are the constructors `True` and `False` of `Bool`
and are constructor patterns, not literals.

### 6.2 Record patterns

In `C { ... }`, each listed field is matched against its pattern; a
field name alone binds the field under its own name; fields not
listed are not constrained. A record type's single constructor is
matched the same way: `User { name }`.

```adt
match i
  | EmailPass { email, hash: _ } -> email
  | Google { gid }               -> gid
  | Apple { sub }                -> sub
```

### 6.3 Nested patterns

A field pattern may itself be a constructor, record or literal
pattern, to any depth:

```adt
type Via = Courier { name: Text } | Mail { tracking: Text }
type Shipment = Pending | Shipped { via: Via }

match s
  | Shipped { via: Courier c } -> c.name
  | Shipped { via: Mail m }    -> m.tracking
  | Pending                    -> "pending"
```

Coverage over nested patterns is decided by the usual matrix algorithm
for ML patterns: the match is exhaustive when no value of the
scrutinee's type escapes every unguarded arm, with the constructors of
each discriminated sum position enumerated and every scalar position
needing a final `_` or binder ([typing.md](typing.md#92-exhaustiveness)).

### 6.4 Named-set patterns

A pattern may name a named constructor set `S` of the scrutinee's sum
`T`: the set `T.S` that the relation variables of `T` declare under
that name, which is one set ([model.md](model.md)). It matches any
value of `T` whose constructor is a member, row or value alike; it
covers every member, and its binder has type `T.S`. Constructor names
in a pattern resolve against the scrutinee's type.

```adt
match i
  | EmailPass e -> e.email
  | OAuth o     -> display_name(o)
```

A constructor added later joins or does not join the set as its `add`
operator states ([evolution.md](evolution.md#42-add-a-constructor)).
If it joins, every such arm covers it; if not, every match that relies
on the set fails its recheck.

### 6.5 Or-patterns

Every alternative of an or-pattern binds the same names. A name bound
to the whole value in each alternative has the row type of the
constructor set of the alternatives:

```text
| Google g | Apple g -> g.user        -- g : Identity.(Google | Apple)
```

A name bound to a field in each alternative has the join of its types
in the alternatives; an or-pattern whose binder types have no join is
refused.

### 6.6 Guards

`| p if g -> v` chooses the arm when `p` matches and then `g`, evaluated
with `p`'s binders, is `true`. If `g` is `false`, matching continues
with the next arm. Coverage is decided without evaluating guards: a
guarded arm covers nothing, so every constructor has an unguarded arm
as well.

```adt
match i
  | Google g if g.gid == "root" -> "staff"
  | Google _                    -> "guest"
  | EmailPass _                 -> "guest"
  | Apple _                     -> "guest"
```

### 6.7 Literal patterns

A literal pattern matches a scalar equal to it under `==`. Text
literals are compared after normalisation to NFC. A literal pattern
takes the type of the scrutinee, as any literal takes its expected
type.

```adt
match c | "US" -> 0 | "CA" -> 1 | _ -> 2
```

### 6.8 Unreachable arms

An arm that can never be chosen is refused at compile time: an arm
whose pattern matches no value left unmatched by the earlier unguarded
arms. A guarded arm after an unguarded arm with the same or a wider
pattern is refused likewise.

Rulings: lang/patterns/nested, lang/patterns/guards, lang/patterns/or-patterns, lang/patterns/literals, lang/patterns/record, lang/patterns/guard-cover/ignore-guarded, lang/patterns/or-binder/ctor-set, lang/wildcard/verbosity/set-patterns, surface/ctor-set/growth/declare

## 7. Navigation

### 7.1 Joins

Related rows are reached by generators joined by predicates. A
reference compares with the key of a row:

```adt
from i in identities, u in users where i.user == key u select u.name
```

Explicit joins remain for every relationship no reference names.

### 7.2 Paths

A field access on a reference follows it. For `k : Key r.S`, `k.f` is
the field `f` of the one row `u` of `r.S` with `key u == k`; it is
shorthand for that generator and predicate:

```adt
from i in identities select i.user.name
-- is
from i in identities, u in users where key u == i.user select u.name
```

The fields reachable through `k` are those of the row type of `r.S`.
A path through a reference to a sum lands on the sum, and its
constructors' fields are reached by `match` (§5.2):

```adt
from r in resets select match r.identity | EmailPass e -> e.email
```

where `identity : Key identities.EmailPass` names one constructor, so
one arm covers it.

Following a key that names no row of its constructor set faults with
`NoSuchRow` (§15). A reference field always names a row; a key passed
as a parameter or built by `r.key(v)` may not, so `users.key(3).name`
faults when no user has key 3.

A path stops at an `Option`. For `manager : Option (Key employees)`,
`e.manager.name` is refused; the option is matched:

```adt
from e in employees
select match e.manager
  | None              -> "none"
  | Some { value: m } -> m.name
```

### 7.3 Backlinks

For a row or key `x` and a relation expression `R` (a relation
variable, constructor relation, constructor set or named set) whose
row type has a field `f` of type `Key t.S` or `Option (Key t.S)`, the
backlink `x.<R.f` is the set of rows of `R` whose `f` references `x`:
for a `Key` field, the rows with `f == key x`; for an `Option` field,
the rows with `f == Some { value: key x }`.

```adt
u.<identities.user
-- is
from i in identities where i.user == key u select i
```

A backlink is a set. It may be ranged over, aggregated, tested with
`exists`, limited with `top`, or held as a nested field of a result.
Used where a single value is expected it is a type error:

```adt
from u in users
select { name: u.name, n: count(i for i in u.<identities.user) }
```

With `type Session = { id: Key sessions, owner: Option (Key users) }`
held by `relation sessions : Session key id generate sequence`,
`u.<sessions.owner` is the sessions whose owner is `Some { value: key
u }`; a session whose owner is `None` is in no user's backlink.

Rulings: lang/nav/joins, lang/nav/paths, lang/nav/backlinks, lang/nav/backlink-card/collection-typed, lang/nav/path-option/option-typed, model/typed-key

## 8. Nested results and the missing side

A record in a result may hold a set, to any depth:

```adt
from u in users
select { name: u.name,
         phones: from d in phones where d.owner == key u select d }
```

There is no outer join. Where a related row is missing, the nested set
is empty; the enclosing row is kept, once. A user with no phones has
`phones` equal to the empty set; a user with two has one result
element holding both.

A nested set is flattened by ranging over it:

```adt
from r in (from u in users select { name: u.name, ds: u.<phones.owner }),
     d in r.ds
select { name: r.name, seen: d.seen }
```

Nested sets compare under `==` as sets. Stored relations hold no
collection fields ([model.md](model.md#12-what-values-are-not)); nested
sets exist only in query values. An implementation may evaluate a
nested result as a fixed number of flat queries; the value is the
nested one.

Rulings: lang/results/nested, lang/outer/nested, model/no-collections

## 9. Grouping

### 9.1 Group into

```adt
from i in identities group i by i.user into g
select { user: g.key, n: count(x for x in g) }
```

`group e by k into g` partitions the current bindings by the value of
`k`. For each distinct value it produces one binding in which:

- `g.key` is that value of `k`;
- `g` is the group: one element per binding of the class, presented as
  the value of `e` for that binding.

A group binder has type `Group K T` for a key of type `K` and elements
of type `T`: `g.key : K`, and `g` is a nonempty `Set T` wherever a set
is expected. A group is never empty. After `into`, the binders introduced before
`group` are out of scope; parameters and enclosing binders remain.
Clauses after `into` (`where`, `from`, `select`) see `g`; a `where`
after `into` filters groups.

An aggregate whose generator ranges over `g` ranges over the bindings
of the group, so equal values of `e` from different bindings each
count (§10.1). Anywhere else, `g` denotes the set of the values of `e`.

The key may be any expression with equality, a record of several keys
included: `group o by { user: o.user, bulk: o.qty > 10 } into g`.

### 9.2 The tag of a sum

For `e` of a sum type `T`, or of a row type of a constructor set of
`T`, `tag(e)` is the constructor of `e` as a value of `Tag T`. `Tag T`
is a sum of nullary constructors, one per constructor of `T`, with the
same names in the same order; it is ordered when `T` is declared
`ordered`. Adding a constructor to `T` adds one to `Tag T`, so a match
on a tag is rechecked like any match.

Per-constructor aggregates are a group by tag:

```adt
from i in identities group i by tag(i) into g
select { kind: g.key, n: count(x for x in g) }
```

No aggregate is computed per arm of a `match` (§10.4).

Rulings: lang/group/into, lang/group/ctor-key/tag-type, query/agg-scope/whole, lang/empty-agg/nonempty

## 10. Aggregates

### 10.1 Aggregates range over bindings

```adt
sum(o.total for o in orders where o.user == key u)
```

An aggregate `agg(v for x1 in e1, ..., xn in en where p)` evaluates
`v` once for every binding of its generators that satisfies `p`, and
combines the values. Equal values from different bindings each count:
two orders of the same total add twice.

A generator of an aggregate may range over any set expression, and
ranges over its elements with set semantics: equal elements of the set
are one element. A generator over a group ranges over the group's
bindings (§9.1). `count`, `min`, `max` and user-defined aggregates
accept a generator over a set built by `select`, and `count` of such a
set counts its distinct values:
`count(a for a in (from p in posts select p.author))` counts distinct
authors ([semantics.md §5.7](semantics.md#57-aggregates),
[prelude.md §7](prelude.md#7-aggregates)).

Only `sum` and `avg` refuse a generator over a set built by a `select`
that is not key-preserving, since such a projection may have merged
equal values; they range over the rows the values come from instead.
The rule is syntactic, applied after normalisation: a generator whose
source normalises to a comprehension is refused unless its `select` is
exactly the binder of a generator over a relation variable, a
constructor relation, a constructor set, a backlink or another such
key-preserving comprehension. A `Set T` parameter is accepted:

```adt
sum(o.total for o in orders)                                        -- each order counts
sum(x.total for x in (from o in orders where o.qty > 1 select o))  -- accepted: selects the binder
sum(t for t in (from o in orders select o.total))                   -- refused
```

### 10.2 The aggregates

| Aggregate | Element type | Result over a nonempty range | Result over a possibly empty range |
| --- | --- | --- | --- |
| `count(v for ...)` | any | `I64`, the number of bindings | `I64`; `0` when empty |
| `sum(v for ...)` | a numeric type `N` | `N` | `N`; `0` when empty |
| `min(v for ...)` | an ordered type `T` | `T` | `Option T`; `None` when empty |
| `max(v for ...)` | an ordered type `T` | `T` | `Option T`; `None` when empty |
| `avg(v for ...)` | `F32` or `F64` | the same type | `Option` of it; `None` when empty |

Integer and decimal `sum` is the exact sum of the values, checked
once against the range of `N` when every value has been added: it
faults (§15) when the exact sum is out of range, whatever the order of
addition. Averages of other
numeric types are taken after an explicit conversion. User-defined
aggregates are declared as in
[interop.md](interop.md#10-user-defined-aggregates) and follow the same
binding rule.

### 10.3 Empty ranges

A range is nonempty when it is a single generator over a group with no
`where`. Every other range may be empty, and `min`, `max` and `avg`
over it return `Option`:

```adt
from o in orders group o by o.user into g
select { user: g.key, top: max(x.total for x in g) }          -- top : I64

from u in users
select { name: u.name,
         top: max(o.total for o in orders where o.user == key u) }   -- top : Option I64
```

### 10.4 Aggregates and match

The arms of a `match` are evaluated per value. An aggregate inside an
arm is an ordinary expression with its own generators, which may
mention the arm's binders; no aggregate ranges implicitly over the
rows that reach an arm. An aggregate over a match's result wraps it:

```adt
count(x for x in (from i in identities
                  select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub))
```

Rulings: syntax/aggregate/comprehension, lang/collection/set-agg/over-binders, lang/empty-agg/nonempty, query/agg-scope/whole

## 11. Recursion

### 11.1 Fix

For a parameter `me: Key users`:

```adt
fix reach = (from f in follows where f.src == me select f.dst)
          union (from r in reach, f in follows where f.src == r select f.dst)
in from u in users where exists r in reach where r == key u select u
```

`fix x1 = q1, ..., xn = qn in e` binds `x1 ... xn` in every `qi` and in
`e`. Their values are the least sets `X1 ... Xn` such that each `Xi`
equals the value of `qi` with every `xj` bound to `Xj`: one
simultaneous least fixpoint. The value of the whole is `e` with the
binders bound to those sets. With one binder this is the least set `X`
equal to `q` with `x` bound to `X`.

Mutual recursion binds its relations together:

```adt
fix even = (from u in users where key u == me select key u)
           union (from o in odd, f in follows where f.src == o select f.dst),
    odd  = from v in even, f in follows where f.src == v select f.dst
in from u in users where exists o in odd where o == key u select u
```

A `fix` binder is a set like any other: its elements are distinct, so a
cycle in the data adds nothing once every element is reached, and the
fixpoint over finite data is reached by iteration from empty sets when
no `qi` builds values absent from the database and parameters.

Recursion is written only with `fix`. A function may not call itself,
directly or through other functions; an implementation must refuse a
cycle of calls among functions.

A `fix` is bounded when no body applies a primitive operation, an
aggregate or a generator of fresh values to a value derived from a
relation of its own stratum, and no body places such a value inside a
constructed record or constructor value. Comparisons and the other
primitives that build no value are exempt. A bounded `fix` adds only
values built from finitely many values present before it runs, so its
least fixpoint is finite and it is total.

A `fix` that is not bounded is partial (§15.2). One whose bodies keep
building new values (`select r + 1`) has an infinite least fixpoint;
its evaluation runs until it reaches a statement limit
([catalog.md](catalog.md#10-configuration)), and the statement is then
rejected with that limit's error, `StatementLimit`. A limit is a
rejection, not a fault, and `try` does not catch it.

### 11.2 Stratified negation and aggregation

The binders of one `fix` form one stratum: within every `qi`, each
`xj` of the same `fix` occurs only at positive positions. The positive
positions are:

- the body `qi` itself;
- both operands of a positive `union`;
- the generator sources and the `select` of a positive comprehension;
- the arms of a positive `match` and the branches of a positive `if`
  (not the scrutinee or condition, except that a `where` condition of
  a positive comprehension is a positive condition);
- inside a positive condition, the range of an `exists` not under
  `not`, and both operands of `and` and `or`;
- the body of a positive `let`, but not its bound expression;
- the bodies and result of a positive inner `fix`.

Every other position is not positive: the operand of `not` (a `not
exists` included), the range and body of an aggregate, the source and
key of `group` and `top`, a field of a record or constructor value, a
primitive's argument, a function body or argument, and a scrutinee. A
`fix` with an occurrence of one of its own binders at a position that
is not positive is refused at compile time
([semantics.md](semantics.md#510-fixpoints)). A set already fully defined,
such as a relation variable, an enclosing query's binder, or another
`fix` none of whose bodies mentions these binders, may be negated or
aggregated inside the bodies: it belongs to a lower stratum. A `match`
is not negation: its arms range over disjoint constructors. The check
is made on the query after functions are inlined (§12.3).

```adt
-- accepted: reach is complete before the outer query negates it
fix reach = (from f in follows where f.src == me select f.dst)
          union (from r in reach, f in follows where f.src == r select f.dst)
in from u in users where not exists r in reach where r == key u select u
```

### 11.3 Recursion is first-order

After function inlining, the `q` of a `fix` contains no function-typed
binder and no unapplied function. A function whose body holds a `fix`
may take function parameters only when every call supplies them so
that inlining removes them; otherwise the call is refused.

Rulings: lang/recursion/fix, lang/recursion/negation/stratified, lang/abstraction/ho-recursion/first-order-recursion

## 12. Functions and views

### 12.1 Schema functions and views

A `fn` names a query or an expression with typed parameters and a
typed result. A function lives in the schema, defined and redefined by
operators in the log and rechecked with every schema change, or in a
bundle, registered and rechecked with it. A view is a function with no
parameters, called with `()`:

```adt
fn logins_of (u: Key users) -> Set Text =
  from i in identities where i.user == u select display_name(i)

fn active () -> Set User = from u in users where exists i in u.<identities.user select u

from u in active(), l in logins_of(key u) select { name: u.name, login: l }
```

A call means the function's body with its parameters bound to the
arguments. Evaluation is call-by-value: every argument is evaluated
before the body, left to right (§15.2). Storing a
view's result is a physical choice that does not change its value
([derived.md](derived.md#2-stored-views)).

### 12.2 Signatures

Every `fn`, `query` and `txn` writes its parameter types and its
result type; bodies are inferred and checked against the signature
([typing.md](typing.md#2-signatures-and-inference)). A function's parameters have
named or row types, anonymous record types with exactly their fields,
`Set` types, or function types; there is no record polymorphism over
"any record with these fields". A `fn` may also return a function.

### 12.3 Higher-order functions are normalised away

A function may take and return functions, and a lambda `fn x -> e`
is a function value:

```adt
fn having (p: Identity -> Bool) -> Set (Key users) =
  from i in identities where p(i) select i.user

having(fn i -> match i | Google _ -> true | EmailPass _ -> false | Apple _ -> false)
```

Before a query is planned, every call is inlined and every application
of a lambda reduced. The result is a first-order query with no
function values. A call is inlined by substituting its arguments only
where they are total; a partial argument stays evaluated once at the
call, before the body, so its fault is reached whether or not the body
uses it. A function type never appears in a relation, a field, a set
element, a type argument, a query result or a parameter of a
registered query, and `==` never applies to a function. An implementation must
normalise every accepted query this way; since functions are not
recursive, normalisation terminates.

Rulings: lang/abstraction/higher-order, lang/nullary-fn, lang/defs-home/schema, lang/defs-home/bundle, lang/inference/signatures, types/infer/bidir-local, lang/record-poly/nominal, evolution/in-log

## 13. Parameters

### 13.1 Declared parameters

Parameters are declared with their types in the signature and referred
to by name in the body. A caller supplies a value of the declared type
for each; no parameter is ever absent or NULL. A parameter of a sum
type is a sum value like any other:

```adt
type Status = Draft | Sent { at: Instant } | Paid { at: Instant }
type Invoice = { id: Key invoices, status: Status, issued: Instant }
relation invoices : Invoice key id generate sequence

query by_status (s: Status, since: Instant) -> Set Invoice =
  from i in invoices where i.status == s and i.issued >= since select i
```

A registered query's parameters have no function types.

### 13.2 Optional filters

A filter a caller may leave off is an `Option` parameter, matched in
the body:

```adt
query search (name: Option Text, after: Option Instant) -> Set Post =
  from p in posts
  where match name  | None -> true | Some { value: n } -> p.title == n
  where match after | None -> true | Some { value: t } -> p.created_at > t
  select p
```

The value is the one the body denotes for the supplied values. An
implementation may plan the query once per pattern of present and
absent `Option` parameters, simplifying each match on a parameter to
its taken arm; each plan must give the value the body denotes.

Rulings: lang/params/declared, syntax/params/names, lang/option-params, types/now/param-only

## 14. Ordering, top n and pagination

### 14.1 Order by

A result is a set and has no order. `order by` presents the final
result of a query in an order:

```adt
from u in users select u order by u.name
```

`order by` and its `take` and `after` are allowed only on the outermost
comprehension of a registered query's body or of an ad hoc statement.
Nested in any other expression they are refused. The result's type is
still `Set T`; the order is the order in which its elements are
delivered ([wire.md](wire.md#52-replies)).

Each ordering expression is evaluated per element of the result, in
the scope of the `select`, and must be a function of the selected
value: it reads only the selected value, or binders whose fields the
selected value holds unchanged. An implementation must refuse an
ordering expression it cannot show to be such a function. An ordering
expression has a type with an order ([model.md](model.md#10-order)); a sum
is ordered only when declared `ordered`. `asc` is the default
direction. Elements equal on every ordering expression are delivered
in an unspecified order among themselves.

### 14.2 Determinate orders

`order by` allows ties (§14.1). Only `take`, `after` and `top` need a
determinate order. They require: distinct
elements must have distinct ordering values. An implementation must
refuse them unless it shows this, which it does when the selected
value is a row and the ordering value includes the row's key, or a
field covered by a key or unique of its constructor set, or when the
selected value is a key and the ordering value includes it.

A compound ordering is a parenthesised list of keys, each with its own
direction: `order by (p.created_at desc, key p)`. Elements compare by
the first key, then by the next on a tie, and so on. Anonymous records
have no order and are never an ordering key.

### 14.3 Top n

```adt
from u in users
select { user: u,
         latest: top 3 by (p.created_at desc, key p)
                   (from p in posts where p.author == key u select p) }
```

`top n by k (q)`, where `k` is one key or a list of keys, is the set of
the first `n` elements of `q` ordered by `k`, or all of them when `q` has fewer. `k` is evaluated in the scope
of `q`'s `select` under the rules of §14.1, and the order must be
determinate (§14.2). `top` is a set and may appear at any depth. Its
result has no order; an enclosing `order by` orders the final result.
`n` is an integer evaluated in the enclosing scope; a negative `n` is a
fault.

### 14.4 Take and keyset pagination

```adt
query page (cursor: Option (Key users)) -> Set User =
  from u in users select u order by u.id take 50 after cursor
```

`take n` delivers the first `n` elements of the ordered result.
`after c`, for one ordering key of type `K`, takes `c : Option K`. For
a list of keys of types `K1 ... Kn`, `c` is an `Option` of an
anonymous record with `n` fields whose types are `K1 ... Kn` in order;
its field names are free and its fields are compared positionally.
With `None` the result starts at its first element; with `Some {
value: v }` it holds only the elements whose ordering value comes
strictly after `v` in the ordering's directions.

```adt
query feed_page (cursor: Option ({ at: Instant, id: Key posts })) -> Set Post =
  from p in posts select p order by (p.created_at desc, key p) take 20 after cursor
```

A client pages by
passing the ordering value of the last element it received. There is
no offset form.

Rulings: lang/order/result-only, lang/nested-order/top-n, lang/keyset, lang/sum-order/declared

## 15. Faults

### 15.1 What a fault does

A fault is an operation applied outside its domain: integer or decimal
overflow, division or remainder by zero, a conversion of a value out
of range of the target type, a negative `top` count, and the other
faulting cases the prelude lists with each operation
([prelude.md](prelude.md)). Each fault is a constructor of the prelude
sum `Fault` (`DivisionByZero`, `ArithmeticOverflow`, `OutOfRange`,
`NoSuchDate`, `InvalidText`, ...). Reaching a statement limit is a
rejection, not a fault (§11.1). `fail` in a write is not a fault
([writes.md](writes.md)). Floating-point arithmetic follows IEEE 754 and
does not fault.

When evaluation reaches a fault, the statement aborts with an error
naming the fault; it returns no rows, and the statement's effects are
not applied ([transactions.md](transactions.md#6-aborts-retry-and-waiting)).
A query's declared result type is unchanged by the operations in it
that can fault: `o.total / o.qty` has the type of its operands.

### 15.2 Evaluation order

Whether a statement faults is fixed by the program, its parameters and
the snapshot, never by the plan. Evaluation is call by value, left to
right within one binding
([semantics.md §5.3](semantics.md#53-binding-and-functions)), and
reaches an expression only where its guards hold:

| Form | Order |
| --- | --- |
| `a and b` | `b` is evaluated only when `a` is `true` |
| `a or b` | `b` is evaluated only when `a` is `false` |
| `match` | the scrutinee first; arms in order; a guard only when its pattern matched; an arm's expression only when the arm is chosen |
| comprehension | clauses in the order written: a `where`, a generator's source or the `select` is evaluated for a binding only when every earlier `where` held for it |
| aggregate | its `where` before its value expression, per binding |
| call | every argument before the body, left to right, whether or not the body uses it |
| other operators, records, constructors | every operand, left to right |

```adt
from o in orders where o.qty > 0 and o.total / o.qty > 10 select o   -- never divides by zero
```

An expression is partial when, after functions are inlined, it may
fault or may not terminate, and total otherwise
([semantics.md §3.4](semantics.md#34-totality)). A `fix` that is not bounded
([semantics.md §5.10](semantics.md#510-fixpoints)) is partial. An implementation may
reorder, push down, hoist, share or drop total expressions freely. It
may move a partial expression only to a position that every guard of
its original position still dominates, and must not drop a partial
expression the program evaluates. Pushing `o.qty > 0` into a scan is
free; evaluating the division before that test is not allowed.

Within one binding, the fault reported is the first one reached in the
order above.

When several bindings fault, the statement reports any one of their
faults ([semantics.md §1.1](semantics.md#11-metanotation)).

### 15.3 Checked operators

Each faulting arithmetic operator and conversion has a checked form
that returns `Option` and never faults: `None` exactly where the
operator would fault, `Some { value: v }` otherwise.

| Checked form | Faulting form |
| --- | --- |
| `checked_add(a, b)` | `a + b` |
| `checked_sub(a, b)` | `a - b` |
| `checked_mul(a, b)` | `a * b` |
| `checked_div(a, b)` | `a / b` |
| `checked_rem(a, b)` | `a % b` |
| `checked_neg(a)` | `-a` |

Their signatures are in [prelude.md §3.2](prelude.md#32-checked-arithmetic),
and the checked conversions are listed with the conversions in
[prelude.md §3.4](prelude.md#34-conversions).

```adt
from o in orders
select match checked_div(o.total, o.qty) | None -> 0 | Some { value: q } -> q
```

### 15.4 Try

`try e` evaluates `e` and never faults. Its type is `Result T Fault`
for `e : T`: `Ok { value: v }` when `e` evaluates to `v`, and `Err {
error: f }` when evaluating `e` reaches the fault `f`, the first in
the order of §15.2; evaluation of `e` stops there and the enclosing
expression continues. When `e` faults in several bindings, `f` is the
fault of any one of them, as for a statement (§15.2). `Fault` is the prelude sum of fault kinds
([prelude.md](prelude.md)). `try` catches faults only: a `fail`, a
serialization failure or any other error that is not a fault is not a
value.

```adt
from r in raws select { line: key r, n: try i32(r.n) }
```

Rulings: types/fault/abort, types/fault/reorder/ordered-guards, types/fault-forms/checked, types/fault-forms/try, types/int-overflow/trap

## 16. Ad hoc statements

An ad hoc statement is query text run by a principal holding the ad hoc
grant ([security.md](security.md#3-grants)), outside any bundle.

- It is checked against the schema current when it runs, with every
  rule of this part, exhaustiveness included, except that `_` and a
  binder may cover the remaining constructors of a sum (§5.4).
- It is not registered, not pinned to a version and never rechecked.
- It may write. An ad hoc write or `transact` block needs, beside
  `adhoc`, the write privileges the statement uses, and every
  constraint holds as for any write. Loads, staging repairs and
  one-off fixes run this way ([syntax.md §7](syntax.md#7-writes-and-transact-blocks)).
- A free lowercase name is a parameter, typed by the descriptor the
  caller supplies for it; a free name the caller supplies no parameter
  for is an error.
- Several items in one ad hoc text form one transaction. Each item is
  a statement; their results are returned in order. The first item
  that fails (a fault, a `fail`, a constraint violation or a
  rejection) ends the text: later items do not run, nothing commits,
  and the error names that item's ordinal.
- Ad hoc text is refused in a transaction that has made a registered
  call, and a registered call is refused in a transaction that has run
  ad hoc text, with `MixedVersions`.
- Ad hoc text may declare `temp relation NAME : T`, a relation outside
  the operator log that has no constraint but its key
  ([syntax.md §3.1](syntax.md#31-kinds-of-file)). It lasts until the
  end of the interactive transaction the text runs in, or, outside
  one, until the end of that one text.
- Text sent on a connection bound to a bundle is checked at the
  current version, not the bundle's pinned version, and its results
  are encoded at the current version, with their tags given by the
  result type descriptor.
- Its results use the same encoding as a registered query's
  ([wire.md](wire.md#52-replies)).

```sh
$ adt repl --role analyst
adt> from i in identities select match i | Google g -> g.gid | _ -> "other"
adt> delete s in logins.Revoked where s.at < cutoff
```

`logins` is the relation [writes.md](writes.md#1-change-sets)
declares; `cutoff` is a parameter of the call.

Rulings: surface/adhoc/scoped, lang/wildcard/verbosity/adhoc-wildcard, evolution/breaks/invisible/registered

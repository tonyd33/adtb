# Typing

This part defines how programs are type checked: which types are
written and which are inferred, the subtype order, variance, the
types of `match` arms and unions, construction and field access,
generic functions, literals, exhaustiveness of patterns, and the
static classification of expressions as total or partial. The types
themselves are defined in [model.md](model.md); the meaning of the
forms checked here is in [queries.md](queries.md) and
[writes.md](writes.md).

Every registered query, schema function, registered transaction and
constraint is checked against the schema when it is defined and again
whenever the schema changes ([evolution.md](evolution.md)). A program
that fails a rule of this part is refused before it runs. An ad hoc
query is checked when it is submitted.

Examples start from version 12 of the running schema, after migration
0011 adds `Apple` and 0012 enables it. Each section's examples are an
independent history.

## 1. Type forms

Beyond the value types of [model.md](model.md#11-kinds-of-type),
checking uses these forms:

| Form | Written | Meaning |
| --- | --- | --- |
| Constructor row type | `Identity.Google` | a value of `Identity` whose constructor is `Google` |
| Constructor-set row type | `Identity.(Google \| Apple)`, `Identity.OAuth` | a value of `Identity` whose constructor is in the set |
| Set type | `Set T` | a set of values of `T` |
| Function type | `T -> U` | a parameter or result of a `fn`, or a lambda |
| Type variable | `a` | a type parameter of a declaration or a generic function |

A named constructor set used at the type level, `Identity.OAuth`,
names the set declared for a relation variable of `Identity`
([model.md](model.md#63-constructor-relations-and-constructor-sets));
it denotes the same constructors as `Identity.(Google | Apple)`. A
schema never declares two different sets of one name for relation
variables of one type, so the name is unambiguous at the type level.
`Identity.(EmailPass | Google | Apple)`, listing every constructor, is
the type `Identity`.

An anonymous result sum ([queries.md
§4.3](queries.md#43-result-sums)) has row types exactly as a declared
sum: for `T = ByEmail { email: Text } | ByGoogle { gid: Text }`,
`T.ByEmail` and `T.(ByEmail | ByGoogle)` are row types, the latter
`T` itself. Two anonymous sums with the same constructors in another
order are one type.

Rulings: lang/binder/ctor-row, lang/patterns/or-binder/ctor-set, spec/anon-sum/row-types/row-types

## 2. Signatures and inference

### 2.1 What is written

The signature of every `query`, `fn` and `txn` is written in full:
each parameter's name and type, and the result type after `->`.
Bodies are inferred.

```adt
type Status = Draft | Sent { at: Instant } | Paid { at: Instant }
type Invoice = { id: Key invoices, status: Status, issued: Instant }
relation invoices : Invoice key id generate sequence

query by_status (s: Status, since: Instant) -> Set ({ invoice: Key invoices }) =
  from i in invoices where i.status == s and i.issued >= since
  select { invoice: key i }
```

Parameters are part of the signature; their types are never inferred
from use. A parameter of a sum type is a value of that sum. An optional
filter is a parameter of type `Option T`, matched in the body
([queries.md](queries.md)).

The result type may be any type: `query n () -> I64 = count(u for u
in users)` is a query. A comprehension is always a set. No binder in a
body needs an annotation.

Ad hoc text has no signature. Its free names are parameters, typed by
the descriptors the caller supplies with the text
([wire.md](wire.md)); a free name with no supplied parameter is a
type error.

Rulings: lang/inference/signatures, lang/params/declared, lang/option-params

### 2.2 Checked and synthesized forms

Checking is bidirectional, with two judgements:

```text
Γ ⊢ e ⇐ T     e checks against the expected type T
Γ ⊢ e ⇒ T     e synthesizes the type T
```

A body checks against its signature's result type. The forms:

```text
Γ ⊢ e ⇒ S    S <: T
──────────────────────────────────────── (sub)
Γ ⊢ e ⇐ T

r : T in Σ
──────────────────────────────────────── (rel)
Γ ⊢ r ⇒ Set T     Γ ⊢ r.C ⇒ Set T.C     Γ ⊢ r.S ⇒ Set T.S

Γ ⊢ e ⇒ Set U
──────────────────────────────────────── (gen)
the generator x in e binds x : U

Γ ⊢ e ⇒ T    Γ ⊢ k ⇒ K
──────────────────────────────────────── (group)
group e by k into g binds g : Group K T

Γ, a : T, b : U ⊢ e ⇒ R
──────────────────────────────────────── (lambda)
Γ ⊢ fn (a: T, b: U) -> e ⇒ T -> U -> R

Γ' ⊢ e ⇐ T    Γ' is Γ with the comprehension's binders
──────────────────────────────────────── (comp)
Γ ⊢ from … select e ⇐ Set T

Γ ⊢ q₁ ⇐ Set T    Γ ⊢ q₂ ⇐ Set T
──────────────────────────────────────── (union)
Γ ⊢ q₁ union q₂ ⇐ Set T

Γ ⊢ p ⇐ Position    Γ_p ⊢ q ⇒ T
──────────────────────────────────────── (at)
Γ ⊢ at commit p { q } ⇒ T
```

(sub) is the only place subsumption applies (§3). In (at), `Γ_p` is
`Γ` with the schema of the version of the past state `q` reads
([derived.md](derived.md#62-naming-a-past-state)); `at instant t { q }`
is the same with `t ⇐ Instant`.

In (group), `g.key ⇒ K`, and `g ⇒ Set T`, nonempty, wherever a set is
expected ([queries.md](queries.md)). `Group K T` is only the type of a
group binder and is never written in a signature; a group passed as an
argument is passed as the nonempty set it denotes, at type `Set T`.

The element types of the binders of `fix X₁ = e₁, …, Xₙ = eₙ` are the
least types `T₁, …, Tₙ` under §3 such that, with each `Xⱼ : Set Tⱼ`,
`eᵢ ⇐ Set Tᵢ` for every `i`. A `fix` with no such types is a type
error.

A call `f(x, y)` of a function value applies its arguments in order. A
schema `fn` with parameters of types `T, U`, named without a call,
synthesizes that curried type; a parameterless one synthesizes
`() -> R` ([queries.md §12.3](queries.md#123-higher-order-functions-are-normalised-away)).

Field access, `key x`, `r.key(...)`, navigation, application of a
declared or prelude function and operators synthesize. A prelude name
with several signatures resolves by its arguments' synthesized types
([prelude.md](prelude.md#1-conventions)).

A record literal, a constructor application, a numeric literal, a
`match`, an `if`, and a function passed to a higher-order function
check against the expected type when there is one. The source query
of `insert r from (q)` checks against `Set` of the row type of the
target `r`, `r.C` or `r.S`.

A call of a generic function takes, among the instantiations of its
type parameters under which every argument meets its parameter type,
the one with the least result type under §3. A call with no such
instantiation is refused, and so is a call whose instantiations have
several minimal result types, incomparable under §3.

```adt
fn oauth_owner (u: Key users) -> Set (Key identities) =
  from i in identities where i.user == u
  select match i | Google g -> key g | Apple a -> key a | EmailPass e -> key e
```

Here `i : Identity`; each arm synthesizes `Key identities.C`, the
arms join to `Key identities` (§5), and the result checks against
`Set (Key identities)`.

Rulings: types/infer/bidir-local, lang/inference/signatures, spec/group/binder-type/binder-only, spec/fix/typing/least, spec/lambda/params/curried, spec/tiers/three-tiers

## 3. Subtyping

The subtype relation `A <: B` is the least reflexive and transitive
relation containing:

| Rule | Condition |
| --- | --- |
| `T.S <: T.S'` | `S` is a subset of `S'` (with `T.S'` = `T` when `S'` is every constructor) |
| `Key r.S <: Key r.S'` | `S` is a subset of `S'` (with `Key r` over every constructor) |
| `Set A <: Set B` | `A <: B` |
| `F A1 ... An <: F B1 ... Bn` | for each parameter, by its variance (§4) |
| `(A -> B) <: (A' -> B')` | `A' <: A` and `B <: B'` |

Nothing else is a subtype of anything else. In particular:

- Distinct declared types are unrelated, whatever their fields.
  A newtype is unrelated to its base type.
- Numeric types are unrelated: `I32` is not a subtype of `I64`.
- Anonymous record types are equal or unrelated. There is no width
  subtyping (a record with more fields is not a subtype of one with
  fewer) and no depth subtyping (`{ k: Key users.Admin }` is not a
  subtype of `{ k: Key users }`). Dropping a field is a written
  projection.
- Two anonymous sum types with different constructors are unrelated.
  Within one anonymous sum, row types are ordered by the first rule,
  as for a declared sum.
- References into different relation variables are unrelated, even
  when the relations have the same element type.

Subsumption never changes a value's representation: a constructor row
is a value of its sum, and a narrower key is a key value.

Rulings: lang/binder/ctor-row, model/ref-subtyping/ctor-set-subsumption, types/sub-records/exact, types/num-mix/explicit, spec/anon-sum/row-types/row-types

## 4. Variance

The variance of each type parameter of a declared sum is derived from
where it occurs in the declaration's fields:

- A parameter that occurs in a field is covariant. Field types are
  covariant positions, since values are immutable, and no field has a
  function type.
- A parameter that occurs in no field is covariant and contravariant:
  any two instantiations at it are related.

```adt
type Option a ordered = None | Some { value: a }     -- prelude; a covariant
type Fallback = { id: Key fallbacks, backup: Option (Key identities.Google) }
relation fallbacks : Fallback key id generate sequence
fn backups () -> Set (Option (Key identities)) =
  from f in fallbacks select f.backup    -- Option (Key identities.Google) <: Option (Key identities)
```

A schema operator that changes a parameter's derived variance can
break dependents; they are rechecked as for any schema change.

Rulings: types/variance/derived

## 5. Joins

### 5.1 Where joins are taken

The arms of a `match` or an `if`, the operands of `union`, and the two
operands of `==`, `!=` and the order comparisons are combined at their
join. When the `match`, `if` or `union` is checked against an expected type, each
arm is checked against that type and no join is taken. When it
synthesizes, its type is the least type `T`, under §3, such that every
arm checks against `T`; for arms that each synthesize a type, `T` is
their join. A form with no such `T` is refused. So
`if c then None else Some { value: 1 }` has type `Option I64`.

### 5.2 The join

The join of two types is their least upper bound under §3, where one
exists:

| Arms | Join |
| --- | --- |
| `T.S` and `T.S'` | `T.(S \| S')`, which is `T` when it is every constructor |
| `Key r.S` and `Key r.S'` | `Key r.(S \| S')` |
| `Set A` and `Set B` | `Set` of the join of `A` and `B` |
| `F A1 ... An` and `F B1 ... Bn` | each parameter that occurs in a field joined; one that occurs in no field taken from either |
| Any other pair, anonymous records included | the type, when the two are equal |

Anonymous records join only when their types are equal: the same
field names at equal field types. Arms of record type whose field
names differ have no join, and the error names each field missing from
an arm; arms whose fields differ in type have no join either. To meet
at a wider field type, the match is checked against a written type
(a signature or a declared record), which checks each arm's fields
against it.
Values of two different sums have no join: there is no top type. Arms
that are meant to stay apart are given constructors of a result sum
([model.md](model.md#64-results)).

```adt fragment
from i in identities select match i
  | Google g    -> g          -- Identity.Google
  | Apple a     -> a          -- Identity.Apple
  | EmailPass e -> e          -- join: Identity

match i
  | Google g    -> { sub: g.gid, team: g.gid }
  | Apple a     -> { sub: a.sub }              -- refused: field team missing
  | EmailPass e -> { sub: e.email }
```

### 5.3 Comparisons

`a == b` and `a != b` are well typed when the operands' types have a
join; they compare at the join. `<`, `<=`, `>` and `>=` additionally
require the join to be ordered ([model.md](model.md#10-order)). Two
`Decimal` operands of different scales compare numerically
([model.md](model.md#33-decimals)). An operand that is a constructor
application, a record literal or a literal takes its type from the
other operand, so `p == None` is well typed for `p : Option T`; no
literal has a `Key` type
([model.md](model.md#81-reference-types)).

```adt
type Visit = { id: Key visits, identity: Key identities }
relation visits : Visit key id generate sequence
from v in visits, g in identities.Google
where v.identity == key g                     -- at Key identities
select v
```

Rulings: types/join/lub-exact-fields, types/sub-records/exact, spec/ctor/type-args/expected-then-fields, spec/join/no-expected/least

## 6. Records, constructors and fields

### 6.1 Nominal typing

Functions take named types, constructor row types and constructor-set
row types. There is no row polymorphism: no parameter type stands for
any record that has a given field. Code shared between types goes
through a sum and a `match`.

```adt
fn email_of (e: Identity.EmailPass) -> Text = e.email
```

Rulings: lang/record-poly/nominal, model/nominal

### 6.2 Construction

- A constructor application `C { f: e, ... }` gives every field of
  `C`, header fields included, and no other. `C` is resolved against
  the expected type when there is one: a constructor of the expected
  declared sum, constructor row type or anonymous result sum.
  Without an expected type, `C` must name a constructor of exactly one
  declared sum in scope; `T.C { ... }` names it explicitly. A
  constructor application synthesizes `T.C`. There are two
  exceptions, both decided by the position of the value:
  - A value written directly as the row of an `insert`, as the
    result of the `select` (or of an arm of a `match` that is that
    result) of the source query of `insert r from (...)`, or as the
    target of an `on delete retag`, may omit the fields its relation
    variable fills from a generator or a default
    ([model.md](model.md#73-generated-keys),
    [derived.md](derived.md#4-defaults)). The target of an `on delete
    retag` and an arm of the `by match` of a split, merge, nest or
    unnest operator may also omit the key fields and the header fields,
    which carry over from the row.
  - The expression after `set` in an `update`, or an arm of a `match`
    directly after `set`, may omit the key fields and the header
    fields of the row's sum; they keep the row's values
    ([writes.md](writes.md#4-retag)). It may also omit the own fields
    its constructor's defaults fill, whether or not it retags the row.
- A generic constructor's type arguments are fixed by the expected
  type or by its fields' types. A type argument fixed by neither is a
  type error naming the parameter: `None` with no expected type, or
  `Err { error: e }` alone, whose fields fix `e` and not `a`.
- A record literal `{ f: e, ... }` checked against a declared record
  type, or against a constructor row type `T.C`, constructs that
  record or constructor. Otherwise it is an anonymous record whose
  type is its fields' types.
- A constructor of an anonymous result sum is only checked, never
  synthesized: it must appear where the result sum is expected, such
  as the arms of a `match` whose type is fixed by the signature.
- `N(e)` for a newtype `N` is a call: it checks `e` against `N`'s base
  type and synthesizes `N`.

```adt
-- given u: User, g: Text
type Import { id: Key imports } = Email { user: Key users, addr: Text, h: Bytes }
                                | Oauth { user: Key users, sub: Text }
relation imports : Import key id generate sequence
insert identities.Google { user: key u, gid: g }    -- literal checked against Identity.Google
insert identities from (
  from x in imports
  select match x
    | Email e -> EmailPass { user: e.user, email: e.addr, hash: e.h }
    | Oauth o -> Google { user: o.user, gid: o.sub })
```

Rulings: spec/ctor/type-args/expected-then-fields

### 6.3 Field access

`x.f` is well typed when `f` is a field of `x`'s type:

| Type of `x` | Fields |
| --- | --- |
| `T` (a sum) | header fields of `T` |
| `T.C` | header fields and the fields of `C` |
| `T.S`, `S` a proper subset of the constructors | header fields, and each field that every constructor of `S` has, at one type |
| Anonymous record | its fields |
| `Key r.S` | the fields of `T.S`, reached by navigation ([queries.md](queries.md#7-navigation)) |
| `Option (Key r.S)` | none; it is matched |

`key x` is defined for any row whose relation is statically known, as
[model.md](model.md#81-reference-types) lists: a binder over a
relation, a constructor set, a backlink, a key-preserving query or a
view returning rows of one relation; a binder over `inserted(R)`,
`deleted(R)` or `prior(R)` in a transition constraint; a `match` arm
on such a row; a
row bound by `let` from a write; a value of a type with a self `Key r`
field. It has type `Key r.S`. Elsewhere it is refused.
`r.key(v)` and `r.key(f: v, ...)` have type `Key r`
([model.md](model.md#81-reference-types)).

Rulings: model/sum-header, surface/shared-fields/header, lang/binder/ctor-row, lang/patterns/or-binder/ctor-set, model/typed-key

## 7. Generic functions

A function, schema-resident or local to a bundle, may take type
parameters, written in brackets after its name. A parameter may require any of four capabilities:

| Bound | Provides | Held by |
| --- | --- | --- |
| `Eq` | `==`, `!=`, set membership | every value type |
| `Hash` | hashing, grouping, hash joins | every value type |
| `Ord` | `<`, `<=`, `>`, `>=`, `min`, `max`, `top ... by`, `order by` | the ordered types ([model.md](model.md#10-order)) |
| `Num` | `+`, `-`, unary `-`, `sum`, integer literals | integer types, `Decimal s`, `F32`, `F64` |

```adt
fn or_else [a] (p: Option a, d: a) -> a = match p | None -> d | Some s -> s.value
fn latest [a: Ord] (xs: Set a) -> Option a = max(x for x in xs)
```

The capabilities are fixed by the engine; there are no user-declared
classes. Every first-order type satisfies `Eq` and `Hash`, so those
bounds hold at every instantiation by a value type. Type parameters range over value types. An instantiation
whose type lacks a required capability is a type error at the call.

`Num` provides `+`, `-`, unary `-` and `sum`. For `Decimal a` and
`Decimal b`, `+` and `-` take the larger scale. `*` and division are
defined per concrete type only, since `Decimal` multiplication changes
the scale: a generic body that applies `*` to a value of a `Num`
variable is refused.

`Span a` carries an implicit `Ord` bound on `a`
([prelude.md](prelude.md#28-span)).

Rulings: types/generics/bounded, types/eq-types/all-first-order, spec/tiers/three-tiers

## 8. Literals and numeric types

A numeric literal takes the numeric type its context expects:

- An integer literal checks against any integer type, any `Decimal s`,
  `F32` and `F64`. No literal checks against a `Key` type.
- A literal with a fractional part checks against any `Decimal s` with
  `s` at least its number of fractional digits, `F32` and `F64`.
- A literal operand of an arithmetic or comparison operator checks
  against the other operand's type: `s.qty + 1` with `qty: I32` is an
  `I32` addition. Against an operand of type `Decimal s`, a literal
  has type `Decimal k`, `k` its own number of fractional digits, and
  the decimal rules of [model.md](model.md#33-decimals) apply.
- A literal that does not fit the expected type, or a fractional
  literal against an integer type, is a compile error.
- With no expected type, an integer literal is `I64`.

With no expected type, a fractional literal is `Decimal k` with `k`
its own number of fractional digits.

Text literals are `Text`, normalized to NFC at compile time. `true`
and `false` are `Bool`.

Operands of arithmetic have one numeric type. There is no implicit
conversion between integer widths, between integers and decimals, or
between either and a float type; conversions are written
([prelude.md](prelude.md#34-conversions)):

```adt
type Sale = { id: Key sales, qty: I32, unit_cents: I64 }
relation sales : Sale key id generate sequence
from s in sales select i64(s.qty) * s.unit_cents
```

Rulings: types/literals/polymorphic, types/num-mix/explicit

## 9. Patterns and exhaustiveness

### 9.1 Binder types

In an arm `| C x -> e` over a scrutinee of type `T` (or `T.S`), `x`
has the constructor row type `T.C`, a subtype of `T`. It can be
returned as a `T`, passed where a `T` is expected, and, when it ranges
over a relation, `key x` has type `Key r.C`.

In an or-pattern `| C x | D x -> e`, `x` has type `T.(C | D)`. A
pattern that names a named constructor set, `| OAuth o -> e`, binds
`o : T.OAuth` and covers every constructor of the set. Named sets are
type-level ([§1](#1-type-forms)), so a named-set pattern matches any
value of the sum, stored or not. A named set takes the payload forms a
constructor takes, over the fields every member shares: `S _`, `S x`,
`S { f: p, g }` where every member has fields `f` and `g`, and bare
`S` only when every member is nullary. A constructor named in a
pattern resolves against the scrutinee's type, never against other
types in scope.

```adt
-- given i: Identity
match i
  | EmailPass e -> e.email
  | OAuth o     -> display_name(o)            -- o : Identity.OAuth
```

A record pattern `C { f: p, ... }` matches the fields of `C` against
subpatterns; a field name alone binds that field. Nested patterns
match value-level sum fields. The meaning of each pattern form is in
[queries.md](queries.md#6-patterns).

Rulings: lang/binder/ctor-row, lang/patterns/or-binder/ctor-set, lang/wildcard/verbosity/set-patterns, spec/pattern/named-set/like-ctor

### 9.2 Exhaustiveness

Every `match` must be exhaustive, and the check is a type check: a
non-exhaustive match is refused at compile time and on every recheck
after a schema change.

- A `match` on a value of a sum `T` must name every constructor of
  `T`, through constructor patterns, or-patterns and named-set
  patterns. On `T.S` it must name every constructor of `S`, and
  naming a constructor outside `S` is a type error.
- `_` does not match a value of a sum. It matches only a scalar: a
  base type or a newtype. Within a constructor pattern, a field of a
  sum type is covered by naming its constructors in some arm; a field
  that no arm discriminates (every arm has a binder or `_` there) is
  covered by that binder or `_`, whatever its type.
- A `match` on a `Key r.S` matches the referenced row by its
  constructor: its arms name the constructors of `S`, and `| C x`
  binds `x : T.C`, the referenced row, as a `match` on the row does.
  `_` is not a pattern for a `Key`. A `match` on a key that is not a
  reference field of a row is partial (§10.1).
- A `match` on a scalar ends with an arm whose pattern is a binder or
  `_`, since literal patterns never cover a scalar type.
- A guarded arm `| C x if p -> e` counts for nothing toward coverage.
  Every constructor also has an unguarded arm, whatever the guards
  say.
- A `match` with nested and record patterns is exhaustive when no
  value of the scrutinee's type escapes every unguarded arm, with each
  subpattern covering as these rules say.

```adt
-- given i: Identity
match i
  | Google g if g.gid == "corp" -> "staff"
  | Google _                    -> "guest"      -- required despite the guard
  | EmailPass _                 -> "guest"
  | Apple _                     -> "guest"
```

Adding a constructor to a sum makes every match on it that does not
cover the new constructor fail its recheck, unless the constructor
joins a named set that the match names. The operator that adds the
constructor supplies the new arm of each schema function, computed
field and match-scoped constraint ([evolution.md](evolution.md)). A
registered query of a bundle gets no arm: it keeps its pinned version.

An arm that can never be reached, because earlier unguarded arms
already cover its constructor, is refused. An alternative of an
or-pattern that matches no value left unmatched by the earlier
unguarded arms and the earlier alternatives of its own arm is refused
likewise.

Rulings: lang/wildcard/scalars-only, lang/patterns/guard-cover/ignore-guarded, lang/wildcard/verbosity/set-patterns, query/compile-time, spec/pattern/redundant-alternative/refused, spec/tiers/three-tiers

### 9.3 Ad hoc queries

In the ad hoc role ([applications.md](applications.md)), `_` may also
match a value of a sum and covers every constructor no other arm
names. Ad hoc queries are never rechecked.

```adt
-- given i: Identity
match i | Google g -> g.gid | _ -> "other"     -- ad hoc only
```

Rulings: lang/wildcard/verbosity/adhoc-wildcard

## 10. Totality and faults

### 10.1 Total and partial expressions

An expression is partial when it may fail to produce a value: it may
fault, or it may not terminate. A statement that does not terminate
ends by a limit rejection, not a fault. Every expression is total or
partial. It is partial when it contains an
operation that can fault (each prelude function's faults are listed in
[prelude.md](prelude.md)), a `fix` that is not bounded ([semantics.md §5.10](semantics.md#510-fixpoints)), which may not
terminate, a path through
a key that is not a reference field of a row (a parameter or a key
built by `r.key(...)`), in a bundle declaring `hide new constructors`
any path through a reference (it faults `NoSuchRow` at a hidden row),
a `match` on a key that is not a reference field of a row (it faults
`NoSuchRow` when the key names no row of its constructor set),
or a call of a function whose body is partial. Hiding is not
transitive: a row of a constructor the bundle knows is visible even
when it references a hidden row
([evolution.md §8.1](evolution.md#81-enabling-a-constructor)). A `fix` that is not bounded
([semantics.md §5.10](semantics.md#510-fixpoints)) is partial even when
its fixpoint happens to be finite. Every other
expression is total. A checked
operator and `add_period(..., Reject)` are total; they return `Option`.

What a fault does, and how evaluation order decides whether one is
reached, is in [queries.md](queries.md#15-faults). Evaluation is call
by value: every argument is evaluated before the body. A registered
query's result type does not mention faults.

Rulings: types/fault/abort, types/int-overflow/trap, spec/tiers/three-tiers

### 10.2 Guards order evaluation

`and`, `or`, the arms of a `match`, guards, and `where` before
`select` evaluate as written, left to right. A partial expression is
evaluated only where every guard before it held:

```adt
type Raw = { id: Key raws, n: I64 }
relation raws : Raw key id generate sequence
from r in raws where r.n != 0 and 1000 / r.n > 10 select r   -- never divides by zero
```

An implementation may reorder, push down, hoist or share total
expressions freely. It may move a partial expression only to a
position that the same guards still dominate.

Rulings: types/fault/reorder/ordered-guards, spec/tiers/three-tiers

### 10.3 Handling a fault

`try e`, for `e : T`, has type `Result T Fault` and is total. `Fault`
is the prelude sum of faults ([prelude.md](prelude.md#25-fault)).
`try` catches faults only, never `fail`
([queries.md](queries.md#154-try)).

```adt
from r in raws select try i32(r.n)       -- : Set (Result I32 Fault)
```

Rulings: types/fault-forms/try, types/fault-forms/checked

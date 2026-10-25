# Data model

This part defines the values a database holds and the types that
classify them: base types, records, sums, newtypes and type
parameters; relation variables and the constructor relations and
constructor sets they expose; keys and references; and the equality
and order of every value type. How expressions over these
types are checked is [typing.md](typing.md). The functions over them
are listed in [prelude.md](prelude.md). What queries over them mean is
[queries.md](queries.md).

Examples start from version 12 of the running schema, after migration
0011 adds `Apple { sub: Text }` joining `OAuth` and 0012 enables it,
unless a section says otherwise, extended for this part with teams,
their members, memberships and visits:

```adt
type Team       = { id: Key teams, name: Text, lead: Key members }
type Member     = { id: Key members, team: Key teams }
type Membership = { user: Key users, team: Key teams, role: Text }
type Visit      = { id: Key visits, identity: Key identities, at: Instant }
relation teams       : Team key id generate sequence
relation members     : Member key id generate sequence
relation memberships : Membership key (user, team)
relation visits      : Visit key id generate sequence
```

Each section's examples are an independent history.

## 1. Values and types

### 1.1 Kinds of type

Every value has exactly one of the following types.

| Form | Written | Identity |
| --- | --- | --- |
| Base type | `I64`, `Text`, `Instant`, ... (§3) | built in |
| Declared sum | `type Stage = Draft \| Sent { at: Instant }` | nominal |
| Declared record | `type User = { id: Key users, name: Text }` | nominal (a one-constructor sum) |
| Newtype | `type Email = new Text` | nominal |
| Instantiation | `Option Text`, `Result I64 Text` | nominal, by the declared sum and its arguments |
| Reference | `Key users`, `Key identities.(Google \| Apple)` | by relation variable and constructor set (§8) |
| Anonymous record | `{ email: Text, n: I64 }` | structural |
| Anonymous sum | `ByEmail { email: Text } \| ByGoogle { gid: Text }` | structural |
| Tag type | `Tag Identity` | derived from its sum (§2.6) |

A declared type is equal only to itself. Two declarations with the same
fields and constructors declare two distinct types, and a value of one
is not a value of the other. A schema operator that renames a declared
type keeps it the same type ([evolution.md](evolution.md)). Nominal
typing holds for every declared type without exception.

Anonymous record and anonymous sum types are equal exactly when they
have the same field names (or constructor names, each with the same
fields) at equal types. Order of fields and constructors does not
affect type equality. They are the types of values that queries build:
projections, nested results, and result sums anonymous to a query.
They are written in the signatures of functions, queries and
transactions. The constructors of a result sum anonymous to a query are
tagged by their order of appearance in the signature text
([wire.md](wire.md)), although that order does not affect type
equality. They are never the type of a field of a type
declaration, nor a relation's element type: every sum or record used
there is itself a declared `type`.

```adt
type Address = { street: Text, city: Text }
type Provider = Google | Apple
type Parcel = { id: Key parcels, ship: Address, bill: Address }
type Handle = { provider: Provider, subject: Text }
relation parcels : Parcel key id generate sequence
```

A function type is not a value type. A `fn` may take and return
functions, and every function is normalised away before a query is
planned ([semantics.md](semantics.md)). A function type never appears
in a relation, a field, a set element, a type argument, a query result
or a parameter of a registered query.

Rulings: model/nominal, model/anon-results/anon-records, model/inline-records, surface/query-scoped, interop/scalar/newtype-only

### 1.2 What values are not

- There is no NULL. An absent value is a constructor of a sum,
  usually `None` of `Option` (§5.2).
- There are no collection-valued fields. No declared type has a field
  whose type is a set, list, bag or map. A many-valued fact is a
  relation whose rows reference their owner:

  ```adt
  type Doc = { id: Key docs, title: Text }
  relation docs : Doc key id generate sequence
  type DocTag = { doc: Key docs, tag: Text }
  relation doc_tags : DocTag key (doc, tag)
  ```

  `Set T` is a type only of query results, of function parameters and
  results, and of fields of anonymous result records (nested results,
  [queries.md](queries.md#8-nested-results-and-the-missing-side)).
- Value types are not recursive. The unfolding of every declared type
  through its fields is finite, so a declaration whose fields reach
  the type itself, directly or through other declarations, is
  refused. A reference names a relation variable, not a type, so
  recursive structure is rows referencing rows:

  ```adt
  type ExprNode { id: Key expr_nodes } = Lit { n: I64 }
                                       | Add { l: Key expr_nodes, r: Key expr_nodes }
  relation expr_nodes : ExprNode key id generate sequence
  ```

Rulings: model/no-collections, model/relation-recursion-only, model/user-params

## 2. Sums

### 2.1 Declaration

A sum is declared once, with every constructor listed as an
alternative:

```adt
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
```

Each constructor has a name, unique within the sum, and a list of
named fields, each with a type. Field names are unique within a
constructor. Constructor names may repeat across different sums.

A sum is closed: it has exactly the constructors its declaration
lists. A constructor is added, removed, split or merged only by a
schema operator ([evolution.md](evolution.md)), never by a
declaration elsewhere.

A sum is disjoint and total: every value of the sum is a value of
exactly one of its constructors. There is no partial or overlapping
specialization and no annotation for one.

Rulings: sum/closed, sum/disjoint-total, surface/sum-decl/alternatives, sum/value, sum/relation

### 2.2 Records

A record type is a sum with one constructor, named after the type:

```adt fragment
type Point = { x: I64, y: I64 }
-- means
type Point = Point { x: I64, y: I64 }
```

There is no separate record type former and no record type that
exists apart from a constructor. A relation of a record type is a
relation-level sum with one constructor, with the same constructor
relations, keys, references, retag and evolution rules as any other
sum. Adding a second constructor to it is an ordinary schema operator.

Rulings: model/record-is-ctor, model/one-ctor-sum

### 2.3 Enumerations

An enumeration is a sum whose constructors have no fields:

```adt
type Severity ordered = Low | Medium | High
```

It has the same `match`, exhaustiveness, order (§10) and evolution
rules as any sum. A nullary constructor may later be given fields by
an operator on that constructor.

Rulings: model/nullary-sum

### 2.4 Header fields

Fields every constructor has are declared once, as a header on the
sum. In the running schema, `id` and `user` are the header of
`Identity` (§2.1).

A header field is a field of every constructor of the sum. A header
field name must differ from every field name of every constructor. A
value of the sum exposes its header fields without a `match`
(`i.user`); constructor fields are reachable only through a value
whose type names the constructor or a constructor set
([typing.md](typing.md#6-records-constructors-and-fields)). A value
keeps its header fields when it changes constructor.

A field is common to the sum only by being in the header. Constructors
that each declare a field of the same name and type do not make that
field a header field.

Rulings: model/sum-header, surface/shared-fields/header

### 2.5 Levels

A sum may sit at two levels, and the same declared type may be used at
both:

- Value level: the type of a field. The field holds one constructor's
  value. Constructor fields at this level are parts of a value: they
  are not keys, references may not occur in them (§8.2), and
  integrity applies to the row that holds the field.
- Relation level: the element type of a relation variable (§6). The
  relation is partitioned into one constructor relation per
  constructor, each with its own uniques and indexes, under one key
  for the whole sum (§7).

Rulings: sum/value, sum/relation

### 2.6 The tag type

Every sum `T` has a derived nullary sum `Tag T` with one constructor
per constructor of `T`, of the same names in the same order. `tag(x)`
for a value `x` of `T` is the `Tag T` constructor naming `x`'s
constructor. `Tag T` gains and loses constructors exactly when `T`
does. `Tag T` is ordered (§10) exactly when `T` is. `Tag` is a prelude
type constructor ([prelude.md](prelude.md#23-tag)).

```adt
-- derived: Tag Identity = EmailPass | Google | Apple
from i in identities group i by tag(i) into g
select { tag: g.key, n: count(x for x in g) }
```

Rulings: lang/group/ctor-key/tag-type

### 2.7 Overlap

Overlap and partiality are relationships to a sum, not kinds of sum.
An entity that holds several kinds of facet at once references a sum
of facets; one facet per kind is a unique per constructor:

```adt
type Factor { id: Key factors, user: Key users } = Password { hash: Bytes }
                                                 | Totp     { secret: Bytes }
relation factors : Factor
  key id generate sequence
  unique (user) in Password
  unique (user) in Totp
```

An optional facet is a relation keyed by a reference to its owner:

```adt
type Part = { id: Key parts, name: Text }
relation parts : Part key id generate sequence
type Manufactured = { part: Key parts, plant: Text }
relation manufactured : Manufactured key part
```

Rulings: sum/overlap/facets-1n, sum/overlap/facets-optional

## 3. Base types

### 3.1 The base set

| Type | Values | Eq | Ord | Num |
| --- | --- | --- | --- | --- |
| `I8`, `I16`, `I32`, `I64` | two's-complement signed integers of 8, 16, 32, 64 bits | yes | yes | yes |
| `Decimal s` | exact decimals of scale `s`, `0 <= s <= 38` (§3.3) | yes | yes | yes |
| `F32`, `F64` | IEEE 754 binary32 and binary64, canonicalized (§3.4) | yes | yes | yes |
| `Text` | Unicode text in NFC (§3.5) | yes | yes | no |
| `Bytes` | byte strings | yes | yes | no |
| `SortKey` | collation sort keys (§3.5) | yes | yes | no |
| `Instant` | points on the UTC time line, microsecond precision (§3.6) | yes | yes | no |
| `Date` | civil dates | yes | yes | no |
| `Time` | civil times of day, microsecond precision | yes | yes | no |
| `DateTime` | civil date and time, no zone | yes | yes | no |
| `ZonedDateTime` | a `DateTime` with a named IANA zone | yes | no | no |
| `Duration` | exact lengths of time in microseconds | yes | yes | no |
| `Period` | calendar lengths: months and days | yes | no | no |

Every other type is a sum, a record or a newtype over these. An
extension cannot add an opaque scalar type.

`Bool` is not a base type. It is the prelude sum `type Bool ordered =
False | True` ([prelude.md](prelude.md#21-bool)); `where` takes a
`Bool`.

Rulings: model/int, model/decimal, model/float, model/text, model/bytes, model/time, model/prelude-bool, types/int-widths/i64, types/int-widths/sized, interop/scalar/newtype-only

### 3.2 Integers

The integer types are `I8`, `I16`, `I32` and `I64`. There are no
unsigned integer types and no integer type wider than 64 bits; wider
or unsigned ranges are `Decimal 0` or a newtype with a domain.

Arithmetic takes operands of one integer type and returns that type.
A result outside the type's range is a fault, as is division by zero;
arithmetic never wraps or saturates. A fault aborts the statement
([queries.md](queries.md#15-faults)). The operators and their checked
forms, which return `Option` instead of faulting, are in
[prelude.md](prelude.md#3-arithmetic).

Rulings: types/int-widths/i64, types/int-widths/sized, types/int-overflow/trap, types/fault-forms/checked

### 3.3 Decimals

`Decimal s` holds the numbers `n / 10^s` for integers `n` with at most
38 decimal digits. The scale `s` is part of the type: `1.0 : Decimal 1`
and `1.00 : Decimal 2` are values of different types. Within one type
each number has one value, so there are no trailing-zero variants.

Scales follow static rules: `+` and `-` give the larger scale of their
operands, rescaling the other exactly, and `*` gives the sum of the
scales. There is no `/`: division is `div`, which names the result
scale and a rounding mode. No operation rounds unless it takes a
rounding mode. Comparison is numeric across any two scales. A result
whose unscaled value exceeds 38 digits is a fault. The operations are
in [prelude.md](prelude.md#33-decimal-functions).

Rulings: types/decimal/scale-typed

### 3.4 Floats

`F32` and `F64` are IEEE 754 binary32 and binary64 with these
refinements:

- Every float result is canonicalized when it is constructed: `-0.0`
  becomes `0.0`, and every NaN becomes the single NaN value. No value
  `-0.0` and no second NaN exists in a table, an index, a result or on
  the wire.
- Arithmetic follows IEEE 754 round-to-nearest-even and does not
  fault: `1.0 / 0.0` is positive infinity and `0.0 / 0.0` is NaN.
- `==` is reflexive: NaN equals NaN. NaN is greater than every other
  float, including positive infinity (§10).

```adt
type Metric = { id: Key metrics, v: F64 }
relation metrics : Metric key id generate sequence
from m in metrics where m.v > 100.0 select m    -- includes rows whose v is NaN
```

Rulings: types/float-eq/nan-greatest, types/float-eq/zero/canon-store

### 3.5 Text

A `Text` value is a sequence of Unicode scalar values in Normalization
Form C. Every operation that constructs text (literals, decoding,
concatenation, case mapping, loading) normalizes its result to NFC, so
the bytes a client sent are not kept when they were not in NFC.
`Bytes` keeps arbitrary bytes. Bytes that are not valid UTF-8 are
never `Text`.

`==` on `Text` is equality of scalar sequences, which on NFC text is
canonical equivalence. The order of `Text` is code-point order. Neither
depends on locale.

The database records the Unicode version its normalization,
case-mapping and segmentation tables follow. Moving to a new Unicode
version is a schema operator that renormalizes stored text and
rebuilds what depends on it ([evolution.md](evolution.md)).

Text has no unqualified length: byte, scalar and grapheme counts are
separate functions. Locale order and insensitive comparison are
functions too, never properties of `Text`, a field or `==`:
`collate(t, c)` maps text to a `SortKey` under a collation `c` whose
name includes its version, and `fold(t)` is case folding
([prelude.md](prelude.md#4-text)). A versioned collation never
changes, so a stored `SortKey` and an index over `collate(...)` keep
their order. Moving a schema to a new collation version is a schema
operator that rebuilds what depends on the old one. A `SortKey` value
compares and orders by its bytes; its encoding on the wire is in
[wire.md](wire.md).

```adt
type Person = { id: Key people, name: Text }
relation people : Person key id generate sequence
from p in people select p order by collate(p.name, "de@icu-74")
```

An index over `collate(f, c)` or `fold(f)` is an index on an
expression ([constraints.md](constraints.md)).

Rulings: types/text-eq/nfc, types/text-eq/unassigned/pin-unicode, types/text-length/explicit, types/collation/sortkey-fn, types/collation/version/versioned-name

### 3.6 Time

All time types use one time scale: every day has 86400 seconds, and a
leap second is not representable. Precision is one microsecond.

| Type | Representation and range |
| --- | --- |
| `Instant` | microseconds since 1970-01-01T00:00:00Z in an `I64` |
| `Date` | a proleptic Gregorian date within the range of `Instant` |
| `Time` | 00:00:00 to 23:59:59.999999 |
| `DateTime` | a `Date` and a `Time` |
| `ZonedDateTime` | a `DateTime` and an IANA zone name |
| `Duration` | microseconds in an `I64` |
| `Period` | a count of months and a count of days, each an `I32` |

`Duration` is exact; `Period` is calendar. They are distinct types and
neither converts to the other implicitly. A `Period` adds to a date or
date-time only through `add_period`, which takes the rule for a day
that does not exist; an `Instant` takes no `Period` at all. The
arithmetic and the zone functions are in
[prelude.md](prelude.md#6-time).

A zoned date-time records the zone a user meant: its value is the
civil date-time and the zone name, never an offset. There is no
offset date-time type; a fixed offset is a field of a declared record.

Zone rules are versioned catalog data. Adopting a new release is a
schema operator ([evolution.md](evolution.md)). After it, the new
rules are in force for every caller, whatever version a bundle pins.
Stored zoned values keep their civil time and zone; the instant they
name is computed under the rules in force, so it can move when a
release changes the rules of their zone.

No expression reads a clock. A query that needs the current time takes
it as a parameter. A write records the current time from a parameter
or from a generated default of the relation it inserts into, such as
`default Revoked.at = generate clock` ([derived.md](derived.md#4-defaults)).

```adt
type Task = { id: Key tasks, due: Instant }
relation tasks : Task key id generate sequence
query due (now: Instant) -> Set ({ task: Key tasks }) =
  from t in tasks where t.due < now select { task: key t }
```

Rulings: model/time, types/time-precision/micros, types/time-scale/smeared, types/durations/split, types/calendar-overflow/explicit, types/zoned/zoned, types/tzdb/catalog, types/now/param-only

### 3.7 Conversions

No numeric type converts to another implicitly
([typing.md](typing.md#8-literals-and-numeric-types)). Every conversion
is a written call, listed with its faults and its checked form in
[prelude.md](prelude.md#34-conversions).

Rulings: types/num-mix/explicit

## 4. Newtypes

A newtype is a distinct nominal type over a base type, optionally with
a domain: a predicate every value must satisfy.

```adt
type Email = new Text where matches(self, "^[^@]+@[^@]+$")
type Cents = new I64
type Contact = { id: Key contacts, email: Email }
relation contacts : Contact key id generate sequence
```

`Email("a@b")` constructs an `Email` from a `Text`; it is a call of the
newtype's name, and never faults: the domain is not checked by
construction. `unwrap(e)` returns the underlying `Text`
([prelude.md](prelude.md#35-newtypes-and-keys)). A `Text` is not an `Email` and an `Email` is not a `Text`:

```adt fragment
insert contacts { email: "a@b" }           -- refused: Text is not Email
insert contacts { email: Email("a@b") }
```

The domain is a constraint on every value of the newtype, checked
where its values are stored ([constraints.md](constraints.md)). A
newtype has the equality, order and hash of its base type and no other
operation. Arithmetic is not lifted to newtypes over numeric types, so
such a newtype is not `Num`; its value is read with `unwrap`.

Rulings: model/newtype, interop/scalar/newtype-only, constraint/domain

## 5. Type parameters

### 5.1 Parametric sums

A declared sum may take type parameters, written lowercase after its
name:

```adt
type These a b = This { value: a } | That { value: b } | Both { this: a, that: b }
```

A field type, a relation's element type or a signature instantiates a
parametric sum with types: `These Text I64`. A relation's element type
has no free type parameters.

Each instantiation of a parametric sum is the same declared sum. Its
constructors are the declared constructors, whose constructor ids are
shared by every instantiation. A schema operator on a
parametric sum reaches every instantiation of it, in every relation
and every dependent query, which are rechecked as for any schema
change ([evolution.md](evolution.md)).

A parameter's variance follows from where it occurs in the declaration
([typing.md](typing.md#4-variance)).

Rulings: model/user-params, model/param-evolution/live, spec/tiers/three-tiers

### 5.2 Library sums

`Option` and `Result` are prelude declarations, not engine built-ins
([prelude.md](prelude.md#22-option-and-result)). They are versioned
with the edition. An optional field is an `Option`, matched like any
sum:

```adt
type Email = new Text where matches(self, "^[^@]+@[^@]+$")
type Profile = { id: Key profiles, email: Email, phone: Option Text }
relation profiles : Profile key id generate sequence
from p in profiles select match p.phone | None -> "-" | Some s -> s.value
```

Rulings: model/user-params, lang/nav/path-option/option-typed

## 6. Relation variables

### 6.1 Declaration

A relation variable holds a set of values of a declared type. The type
and the variable are declared apart:

```adt
type Country = { code: Text, name: Text }
relation countries         : Country key code
relation retired_countries : Country key code
```

A type may be the element type of any number of relation variables,
and of fields elsewhere, unless it declares a key field of its own
relation (§7.1). Each relation variable has its own constructor
relations, key, uniques, indexes and references. Its element type is
a declared sum (or record) or an instantiation of one.

Rulings: model/set-of-type, surface/extent/separate

### 6.2 Sets

A relation is a set: it never holds two equal rows. Every result of a
comprehension is a set too, deduplicated by `==` (§9). In particular a
`match` whose arms project values that coincide yields each value
once:

```adt
from i in identities select match i
  | EmailPass e -> e.user
  | Google g    -> g.user
  | Apple a     -> a.user          -- each user once
```

Rulings: model/sets, model/dup-arms/dedupe

### 6.3 Constructor relations and constructor sets

For a relation variable `r` whose element type is a sum `T`:

- `r` is the whole sum.
- `r.C` is the constructor relation of constructor `C`: the rows of
  `r` whose constructor is `C`.
- `r.(C | D)` is a constructor set: the union of `r.C` and `r.D`.
- `set S of r = C | D` declares a named constructor set `S` of `r`,
  used as `r.S`. Named sets belong to a relation variable, not to its
  type. A constructor joins a named set only by being listed in it, or
  by the operator that adds the constructor stating that it joins
  ([evolution.md](evolution.md)). There is no complement form.

`r` is the disjoint union of its constructor relations. A constructor
set is always written by listing constructors or by naming a set.

Two relation variables of one type may not declare named sets of the
same name with different constructors: a named set used at the type
level (`Identity.OAuth`, [typing.md](typing.md#1-type-forms)) must
denote one set of constructors. A declaration or operator that would
make them differ is refused.

Rulings: sum/relation, surface/ctor-set/named, surface/ctor-set/list

### 6.4 Results

A query result may be of the same kind as a stored relation: a set
whose element type may be a sum. A result sum anonymous to a query is
written in its signature, and its constructors are matched downstream
like those of a stored sum, with the same exhaustiveness:

```adt
fn sign_ins () -> Set (Local { id: Key identities, email: Text }
                     | Remote { id: Key identities, provider: Text }) =
  from i in identities select match i
    | EmailPass e -> Local  { id: key e, email: e.email }
    | Google g    -> Remote { id: key g, provider: "google" }
    | Apple a     -> Remote { id: key a, provider: "apple" }

from l in sign_ins().Remote select l.provider
```

Rulings: model/sum-results, surface/query-scoped

## 7. Keys

### 7.1 One key per relation

Identity is a declared key. Every relation variable declares exactly
one key, `key f` or `key (f, g, ...)`. No two rows of the relation
agree on the key. There is no row identity apart from the key.

The key ranges over every constructor of the relation: a relation-level
sum has one key, carried by every constructor. The key names fields
that the element type declares: header fields, or fields of its only
constructor. A value keeps its key when it changes constructor, so
references to it stay valid across a retag
([writes.md](writes.md#4-retag)).

A key field may have any type. A key field that identifies its row by
a value of no other meaning has type `Key r` for its own relation `r`,
as `id` does in this excerpt of the running schema:

```adt fragment
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
```

A type with a field of type `Key r` that is the key of `r` is the
element type of `r` only; no other relation variable may hold it. A
type whose key fields are ordinary values (`code: Text`) may be the
element type of several relation variables, each with its own key
values.

`Key r` is a distinct type for every relation variable `r`, whatever
its key fields are. Its representation is the value of the key fields:
`r.key(v)` builds a `Key r` from that value and `value(k)` reads it
back (§8.1). `countries.key("PT")` is a `Key countries`, not a `Text`.
A relation keyed by a field of type `Key s` still has its own `Key r`,
distinct from `Key s`: `manufactured` (§2.7) is keyed by `part: Key
parts`, and `Key manufactured` is not `Key parts`.

Rulings: model/declared-key, model/one-key, types/id-type/key-type

### 7.2 What a key may be made of

Any value type may form a key:

| Key form | Example |
| --- | --- |
| Composite | `relation memberships : Membership key (user, team)` |
| A sum-valued field | `key (provider, subject)` with `provider: Provider` |
| A reference | `relation order_lines : OrderLine key (order, n)` with `order: Key orders` |
| A record-valued field | `key span` with `span: DateRange`, `type DateRange = { from: Date, to: Date }`, equal to the composite key of its fields |

```adt
type Membership = { user: Key users, team: Key teams, role: Text }
relation memberships : Membership key (user, team)
```

A facet of an owner is keyed by the owner and the facet's tag:
`key (owner, kind)`.

Rulings: model/composite, model/sum-valued-key, model/ref-in-key, model/record-key

### 7.3 Generated keys

A relation may generate its key, and fill other fields from defaults.
Generators and defaults are clauses of the relation variable; a type
never carries one, so constructing a value is pure. An insert that
does not supply a generated field gets a value drawn from the
generator:

```adt
type Order = { id: Key orders, buyer: Key users }
type Token = { id: Key tokens, secret: Bytes }
relation orders : Order key id generate sequence
relation tokens : Token key id generate uuidv7
```

| Generator | Fills | Promise |
| --- | --- | --- |
| `generate sequence` | a key field; the key is a 64-bit integer | distinct values; gaps after aborts; no order across transactions |
| `generate uuidv7` | a key field; the key is an RFC 9562 UUIDv7, represented as a `Uuid` | distinct values |
| `generate clock` | an `Instant` field, through `default f = generate clock` | the wall-clock time at which the statement applies, one value for every row of the statement |

A generated key field has type `Key r` whatever the representation.
The schema chooses the generator. A sequence and a UUIDv7 promise only
distinct values, in neither commit order nor start order. A UUIDv7
needs no shared counter and can be minted outside the database. The
engine applies no transformation to generated values: a sequence
reveals how many keys were issued and a UUIDv7 its creation time. A
`clock` value promises no order relative to the serialization order of
transactions.

A generated field is still a field the type declares: it is stored,
compared and exported with the row, and an insert or a load may supply
it. A generated `sequence` key differs from every value the generator
has issued and from every key value that any committed row of the
relation has held, supplied values included.

`key id` names a field the element type declares; it never adds a
field. Other defaults are in [derived.md](derived.md#4-defaults).

Rulings: model/declared-key, types/id-type/key-type, types/keygen/sequence, types/keygen/uuidv7, types/keygen/promise/distinct, types/keygen/import/advance, types/keygen/leak/live, types/now/param-only, spec/writes/sequence-distinct/issued-and-held

## 8. References

### 8.1 Reference types

A reference holds the target row's key, typed by the constructor set
it may point at:

| Type | Points at |
| --- | --- |
| `Key r` | any row of relation `r` |
| `Key r.C` | a row of constructor `C` of `r` |
| `Key r.(C \| D)`, `Key r.S` | a row of any constructor of the set |

```adt
type Visit      = { id: Key visits, identity: Key identities, at: Instant }
type Recovery   = { id: Key recoveries, identity: Key identities.EmailPass, token: Bytes }
type OAuthToken = { id: Key oauth_tokens, identity: Key identities.OAuth }
relation visits       : Visit key id generate sequence
relation recoveries   : Recovery key id generate sequence
relation oauth_tokens : OAuthToken key id generate sequence
```

A reference names a constructor set of a relation variable, never of a
type. The field holds the target key as a visible value: it is
readable, printable and comparable with `==`. A key value is obtained
in two ways:

- `key x` is defined for any row whose relation is statically known:
  a binder over a relation, a constructor set or a backlink; a
  binder over a key-preserving query (one that selects the binder
  itself) or over a view returning rows of one relation; a binder
  over `inserted(R)`, `deleted(R)` or `prior(R)` in a transition
  constraint ([constraints.md](constraints.md#61-reading-the-change));
  a `match` arm on such a row; a row bound by `let` from a write; and
  a value of a type with a self `Key r` field. Its type is `Key r.S` for the
  constructor set `x` belongs to. Elsewhere `key x` is refused.
- `r.key(v)` builds the key of `r` from the value of its key field,
  and `r.key(f: v, g: w)` from the values of the fields of a composite
  key. Its type is `Key r`. For a `sequence` key `v` is an `I64`; for
  a `uuidv7` key it is a `Uuid`, built from text by `uuid(t)`
  ([prelude.md](prelude.md#27-uuid)). `value(k)` reads the value back.
  A built key need not name an existing row; a reference field that
  holds it is checked when it is written.

No literal checks against a `Key` type.

```adt
query visits_of (id: Uuid) -> Set ({ visit: Key visits }) =
  from v in visits where v.identity == identities.key(id) select { visit: key v }

identities.key(uuid("0190a4b2-7c3e-7d4f-9a21-5b6c7d8e9f01"))
query role_of (u: Key users, t: Key teams) -> Set ({ role: Text }) =
  from m in memberships where key m == memberships.key(user: u, team: t)
  select { role: m.role }
```

`Key r.S` widens implicitly to `Key r.T` whenever `S` is a subset of
`T`, and `Key r` is `Key r` over every constructor. Narrowing is a
`match` on the referenced row, by its constructor:

```adt
from v in visits
select match v.identity | EmailPass e -> e.email | Google _ -> "-" | Apple _ -> "-"
```

Every reference is a foreign key to its constructor set: a
`Key identities.EmailPass` field may hold only the key of an existing
`EmailPass` row of `identities`, so a recovery for a Google identity is
unrepresentable. When a retag moves a target out of a referrer's
constructor set, that referrer is checked like any other reference
([constraints.md](constraints.md)). The action taken when a target is
deleted is written on the field ([writes.md](writes.md#7-delete-actions)).

Rulings: model/typed-key, model/ref-subtyping/ctor-set-subsumption, fk/to-sum, fk/to-ctor, fk/to-subset, fk/retag/recheck, surface/ref/ref-type

### 8.2 Where references may occur

A reference is a direct field of a constructor (or a header field) of
a relation's element type, of type `Key r.S` or `Option (Key r.S)`. No
reference occurs deeper inside a field's value: not in any other
value-level sum's constructor fields, not in a record-valued field. A
sum of references is a relation-level sum whose constructors each
hold one:

```adt
type Doc = { id: Key docs, title: Text }
relation docs : Doc key id generate sequence
type Ownership { doc: Key docs } = Owned  { user: Key users }
                                 | Shared { team: Key teams }
relation ownerships : Ownership key doc
```

Anonymous result records may hold references; they are values, not
constrained fields.

Rulings: model/refs-top-level, lang/nav/path-option/option-typed

### 8.3 Cycles

The reference graph of a schema may contain cycles, including
self-references. References are checked at the end of the multiple
assignment that applies a change set, so rows that reference each
other are inserted in one change set:

```adt
type Team   = { id: Key teams, name: Text, lead: Key members }
type Member = { id: Key members, team: Key teams }
relation teams   : Team key id generate sequence
relation members : Member key id generate sequence

transact
  let t = insert teams { id: teams.key(1), name: "core", lead: members.key(9) }
  let m = insert members { id: members.key(9), team: key t }
  return key t
```

Rulings: model/deferred-check

## 9. Equality

Every value type has `==`, derived structurally. No declaration can
remove it.

| Type | `a == b` when |
| --- | --- |
| Integers, `Decimal s`, `Duration` | equal numbers |
| `F32`, `F64` | equal numbers, or both NaN (§3.4) |
| `Text`, `Bytes`, `SortKey` | equal sequences |
| `Instant`, `Date`, `Time`, `DateTime` | the same point or civil value |
| `ZonedDateTime` | the same civil date-time and the same zone name |
| `Period` | the same month count and the same day count |
| Newtype | equal underlying values |
| Sum or record | the same constructor and equal fields |
| Anonymous record | equal fields |
| `Key r.S` | equal key values |
| `Set T` | the same elements |

`==` is substitutive: `a == b` implies `f(a) == f(b)` for every
function `f` of the language. Every type's values are canonical so
that this holds: one zero and one NaN, NFC text, scale fixed by the
type. Consequently which of two equal values deduplication keeps, and
whether the engine substitutes one for the other, is never observable.

`!=` is the negation of `==`. Equality between different scales of
`Decimal` is numeric (§3.3); between any other two distinct types it is
a type error ([typing.md](typing.md#5-joins)).

Rulings: types/eq-types/all-first-order, types/eq/leibniz, types/float-eq/nan-greatest

## 10. Order

A type is ordered (`Ord`) when the table in §3.1 says so, or when it is
one of:

- A sum or record declared `ordered`, every field type of which is
  ordered. A sum that is not declared `ordered` has no order, and
  sorting by it, `<` on it, `min` or `max` over it, or a range over it
  are type errors. An enumeration is ordered only when declared so.
- An instantiation of an `ordered` parametric sum, when every field
  type is ordered at that instantiation. `Bool`, `Option a` and
  `Span a` are declared `ordered` ([prelude.md](prelude.md#2-types)).
- A newtype over an ordered base type.
- `Key r.S`, when the key's type is ordered.

Orders are:

| Type | Order |
| --- | --- |
| Integers, `Decimal s`, `Duration` | numeric |
| `F32`, `F64` | numeric from `-inf` to `inf`, then NaN |
| `Text` | code point, lexicographic |
| `Bytes`, `SortKey` | unsigned byte, lexicographic; a prefix sorts first |
| `Instant`, `Date`, `Time`, `DateTime` | chronological |
| Ordered sum or record | constructor declaration order, then header fields, then the constructor's fields, each left to right |
| `Key r.S` | the order of the key value |

`ZonedDateTime` and `Period` have no order; a zoned value sorts by
`to_instant(z)`. Anonymous records and anonymous sums have no order;
a compound sort key is a list of keys
([queries.md](queries.md#14-ordering-top-n-and-pagination)).
Where a schema change places a new constructor in declaration order is
stated in [evolution.md §4.2](evolution.md#42-add-a-constructor).

```adt
type Stage ordered = Draft | Sent { at: Instant } | Paid { at: Instant, ref: Text }
-- Draft < Sent { at: t1 } < Paid { at: t0, ref: "a" } for any t0, t1
```

Rulings: lang/sum-order/declared, model/enum-order/derived-order, spec/evolution/ctor-order/placed, spec/tiers/three-tiers

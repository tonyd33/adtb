# Prelude

This part lists the prelude: the types and functions every module
reads without qualification. For each function it gives the
signature, what the function returns, and when it faults. The base
types the functions act on are defined in [model.md](model.md#3-base-types).
How calls are type checked is in [typing.md](typing.md). What a fault
does is in [queries.md](queries.md#15-faults).

The prelude is versioned with the edition. A schema must not declare a
type or function whose name the prelude declares, nor a type or type
parameter named by one of the built-in type and bound names `Set`,
`Key`, `Group`, `Eq`, `Ord`, `Hash` and `Num`; a constructor or field
may have one of these names. The one exception is `Principal`, which
a schema may declare in its first migration (§2.6). Prelude constructors
have fixed constructor ids, listed per edition in
[catalog.md](catalog.md#4-error-sums).

## 1. Conventions

- A signature is written `name (p: T, ...) -> U`. A type variable
  with a bound is written `a: Ord`. `N` stands for any numeric type:
  `I8`, `I16`, `I32`, `I64`, `Decimal s`, `F32`, `F64`. `Int` stands
  for any integer type. `Float` stands for `F32` or `F64`.
- A function is total unless its row in the table names a fault.
  Partial means an expression may fail to produce a value: it may
  fault, or it may not terminate, as an unbounded `fix` may; a
  statement that does not terminate ends by a limit rejection. A
  fault is a constructor of `Fault` (§2.5).
- A name with several signatures in this part is overloaded: the
  arithmetic and comparison operators, `++`, the conversions,
  `checked_*`, `abs`, `bytes_len`, `text`, `add_period`, and `covers`,
  which takes a span or a point as its second argument. A call
  resolves to the signature that matches its arguments' synthesized
  types, taken left to right; a literal argument takes the type its
  position requires once another argument fixes it, so `s.qty + 1`
  with `qty: I32` adds two `I32` values. A call that matches no
  signature, or more than one, is a type error.
- An argument documented as "constant" must be a literal, or a
  constructor written in place, so that its value is known when the
  call is checked.
- Arguments are evaluated before the call, left to right
  ([queries.md](queries.md#15-faults)).

Rulings: none

## 2. Types

### 2.1 Bool

```adt
type Bool ordered = False | True
```

`true` and `false` are literal spellings of `True` and `False`.
`False < True`.

| Form | Meaning |
| --- | --- |
| `a and b` | `True` when both are; `b` is evaluated only when `a` is `True` |
| `a or b` | `True` when either is; `b` is evaluated only when `a` is `False` |
| `not a` | the other constructor |
| `if c then a else b` | `match c \| True -> a \| False -> b` |

`if` is typed as the `match` it stands for: its arms join as the arms
of a `match` do ([typing.md](typing.md#5-joins)).

### 2.2 Option and Result

```adt
type Option a ordered = None | Some { value: a }
type Result a e = Ok { value: a } | Err { error: e }
```

`None < Some { ... }`; two `Some` values order by `value`. `Option a`
is ordered when `a` is. `Result` has no order.

### 2.3 Tag

`Tag T`, for a sum `T`, is the derived nullary sum with one
constructor per constructor of `T`, of the same names in the same
order. `tag(e)` gives the constructor of `e` as a `Tag T` value
([model.md](model.md#26-the-tag-type),
[queries.md](queries.md#92-the-tag-of-a-sum)).

### 2.4 Rounding and Overflow

```adt
type Rounding = HalfEven | HalfUp | HalfDown | Up | Down | Ceiling | Floor
type Overflow = Clamp | Reject
```

| Rounding | Rounds to |
| --- | --- |
| `HalfEven` | the nearest value; a tie goes to the even last digit |
| `HalfUp` | the nearest value; a tie goes away from zero |
| `HalfDown` | the nearest value; a tie goes toward zero |
| `Up` | away from zero |
| `Down` | toward zero |
| `Ceiling` | toward positive infinity |
| `Floor` | toward negative infinity |

`Overflow` names what a calendar operation does with a civil time that
does not exist (§6.3, §6.4).

### 2.5 Fault

```adt
type Fault = DivisionByZero
           | ArithmeticOverflow
           | OutOfRange
           | NotFinite
           | NoSuchDate
           | UnknownZone
           | InvalidText
           | InvalidSpan
           | NegativeCount
           | NoSuchRow
           | FunctionFailed
```

Its type and constructor ids are listed in
[catalog.md §4.6](catalog.md#46-constructor-ids-of-engine-defined-sums).

| Constructor | Raised by |
| --- | --- |
| `DivisionByZero` | `/`, `%` or `div` with a zero divisor |
| `ArithmeticOverflow` | an arithmetic result outside its type's range, integer, decimal or time |
| `OutOfRange` | a conversion or constructor argument outside the target's range |
| `NotFinite` | converting NaN or an infinity to an integer or decimal |
| `NoSuchDate` | `zoned(..., Clamp)` or `add_period(..., Clamp)` when no valid time is reached within the range of the type |
| `UnknownZone` | a zone name the database's zone rules do not define |
| `InvalidText` | text that does not satisfy what the function requires of it |
| `InvalidSpan` | a span whose start is after its end |
| `NegativeCount` | `top n` with `n < 0` |
| `NoSuchRow` | following a key by a path when the key names no row of its constructor set |
| `FunctionFailed` | a Wasm function that traps or exhausts its fuel or memory, or a host function that returns an error or panics |

`try e` catches a fault and gives it as the `Err` of a `Result T
Fault`; it does not catch `fail` or any other error
([queries.md](queries.md#154-try)). A statement that exceeds a
configured limit is rejected, not faulted, and `try` does not catch
it. A fault that aborts a statement
reaches the caller as `Rejected { reason: Faulted { fault: f } }`
([catalog.md](catalog.md#4-error-sums)). `Fault` is not ordered.

### 2.6 Error, audit sums and Principal

`Error`, its reason sums, `Notice` and the other error sums are prelude
sums defined in [catalog.md](catalog.md#4-error-sums). `Origin` and
`Outcome`, the types of what an audited event ran and how it ended,
are prelude sums defined in [security.md](security.md#82-the-audit-relation):

```adt
type Origin = Registered { bundle: Text, query: I32 }
            | Function   { name: Text }
            | AdHoc      { text: Bytes }
type Outcome = Succeeded | Failed { error: Error }
```

`Position`, a commit position on one history, is a prelude record;
`TxnId`, a transaction's id, and `Report`, the encoded result rows of a
violated constraint, are prelude newtypes. All three are defined in
[catalog.md](catalog.md#42-reasons):

```adt
type Position = { epoch: I64, lsn: I64 }
type TxnId = new I64
type Report = new Bytes
```

`RejectedRow`, the row type of a relation that receives an insert's
rejects, is a prelude record defined in
[interop.md](interop.md#45-rejects).

`Principal` is the type of the caller; `caller` always has the type
`Principal` names. A schema may declare its own `Principal`, only in
its first migration. That declaration belongs to the root namespace,
resolves as `Principal` from every module and every bundle, takes ids
from the operator log and changes only by operators. A schema that
never declares one has the prelude's default
`type Principal = EndUser { name: Text } | Service { name: Text } | Anonymous`,
whose type and constructor ids 37 to 40
([catalog.md](catalog.md#46-constructor-ids-of-engine-defined-sums))
are used only by the default
([security.md](security.md#11-the-principal-sum)).

### 2.7 Uuid

```adt
type Uuid = new Bytes where bytes_len(self) == 16
```

| Function | Result | Faults |
| --- | --- | --- |
| `uuid (t: Text) -> Uuid` | the UUID written in the canonical 8-4-4-4-12 hexadecimal form, either case | `InvalidText` for any other text |
| `text (u: Uuid) -> Text` | the canonical form, lowercase | none |

A `uuidv7` key is represented as a `Uuid`
([model.md](model.md#73-generated-keys)).

```adt
identities.key(uuid("0190a4b2-7c3e-7d4f-9a21-5b6c7d8e9f01"))
```

### 2.8 Span

```adt
type Span a ordered = { start: a, end: a }
```

A `Span a` is the half-open interval from `start` (included) to `end`
(excluded) of an ordered type `a`. Building a span whose `start` is
after its `end` faults with `InvalidSpan`. A span is built only by
`span`; a record literal or constructor application of `Span` is
refused. The parameter carries an implicit `Ord` bound: `Span a` is
well formed only for an ordered `a`. A span whose `start` equals its `end` is empty. Spans
order by `start`, then `end`.

| Function | Result |
| --- | --- |
| `span [a: Ord] (start: a, end: a) -> Span a` | the span; faults `InvalidSpan` when `start > end` |
| `start [a: Ord] (s: Span a) -> a` | `s.start` |
| `end [a: Ord] (s: Span a) -> a` | `s.end` |
| `overlaps [a: Ord] (x: Span a, y: Span a) -> Bool` | `x.start < y.end and y.start < x.end` |
| `covers [a: Ord] (x: Span a, y: Span a) -> Bool` | `x.start <= y.start and y.end <= x.end` |
| `covers [a: Ord] (x: Span a, t: a) -> Bool` | `x.start <= t and t < x.end` |

```adt
type Booking = { id: Key bookings, room: Text, during: Span Instant }
relation bookings : Booking key id generate sequence
from a in bookings, b in bookings
where a.room == b.room and key a != key b and overlaps(a.during, b.during)
select { first: key a, second: key b }
```

Rulings: model/prelude-bool, model/user-params, lang/group/ctor-key/tag-type, types/calendar-overflow/explicit, types/fault-forms/try, types/fault/abort

## 3. Arithmetic

### 3.1 Operators

| Operator | Signature | Faults |
| --- | --- | --- |
| `a + b`, `a - b` | `Int -> Int -> Int`, one type | `ArithmeticOverflow` |
| `a * b` | `Int -> Int -> Int`, one type | `ArithmeticOverflow` |
| `a / b` | `Int -> Int -> Int`, truncating toward zero | `DivisionByZero`; `ArithmeticOverflow` for the minimum value divided by `-1` |
| `a % b` | `Int -> Int -> Int`, the sign of `a` | `DivisionByZero` |
| `-a` | `Int -> Int` | `ArithmeticOverflow` for the minimum value |
| `a + b`, `a - b` | `Decimal s -> Decimal t -> Decimal (max(s, t))` | `ArithmeticOverflow` above 38 digits |
| `a * b` | `Decimal s -> Decimal t -> Decimal (s + t)`; refused when `s + t > 38` | `ArithmeticOverflow` above 38 digits |
| `-a` | `Decimal s -> Decimal s` | none |
| `a + b`, `a - b`, `a * b`, `a / b` | `Float -> Float -> Float`, one type | none; IEEE 754 results, canonicalized |
| `-a` | `Float -> Float` | none |

There is no `/` and no `%` on decimals. Comparison operators are in
[typing.md](typing.md#53-comparisons).

### 3.2 Checked arithmetic

Each checked form returns `None` exactly where its operator faults and
`Some { value: v }` otherwise. It never faults.

| Function | Signature |
| --- | --- |
| `checked_add`, `checked_sub`, `checked_mul` | `(a: N, b: N) -> Option N` for an integer or decimal `N`, with the operator's result type |
| `checked_div`, `checked_rem` | `(a: Int, b: Int) -> Option Int` |
| `checked_neg` | `(a: Int) -> Option Int` |

```adt
type Raw = { id: Key raws, n: I64 }
relation raws : Raw key id generate sequence
from r in raws select match checked_div(1000, r.n) | None -> 0 | Some q -> q.value
```

### 3.3 Decimal functions

| Function | Result | Faults |
| --- | --- | --- |
| `div (a: Decimal s, b: Decimal t, u: constant I64, r: Rounding) -> Decimal u` | `a / b` rounded to scale `u` by `r` | `DivisionByZero`; `ArithmeticOverflow` |
| `round (a: Decimal s, u: constant I64, r: Rounding) -> Decimal u`, `u < s` | `a` rounded to scale `u` by `r` | none |
| `rescale (a: Decimal s, u: constant I64) -> Decimal u`, `u >= s` | `a` at scale `u`, exactly | `ArithmeticOverflow` above 38 digits |
| `abs (a: N) -> N` | the absolute value | `ArithmeticOverflow` for the minimum integer value |

```adt
type Quote = { id: Key quotes, price: Decimal 2, rate: Decimal 4 }
relation quotes : Quote key id generate sequence
from q in quotes select round(q.price * q.rate, 2, HalfEven)    -- : Set (Decimal 2)
```

### 3.4 Conversions

No numeric type converts to another implicitly. Each conversion that
can fault has a checked form named `checked_` followed by its name
(`checked_i32`, `checked_decimal`, ...), with the same arguments,
returning `Option`.

| Function | From | Result | Faults |
| --- | --- | --- | --- |
| `i8`, `i16`, `i32`, `i64` `(x)` | any integer type | the integer | `OutOfRange` |
| `i8`, `i16`, `i32`, `i64` `(x, r: Rounding)` | `Decimal s`, `Float` | `x` rounded by `r` | `OutOfRange`; `NotFinite` |
| `decimal (x, s: constant I64)` | any integer type | `x` at scale `s` | `OutOfRange` above 38 digits |
| `decimal (x, s: constant I64, r: Rounding)` | `Float` | `x` rounded to scale `s` by `r` | `OutOfRange`; `NotFinite` |
| `f32 (x)`, `f64 (x)` | any integer type, `Decimal s`, the other float type | nearest value, ties to even | `f32` only: `OutOfRange` beyond the `F32` range |

### 3.5 Newtypes and keys

| Function | Result | Faults |
| --- | --- | --- |
| `N (x: B) -> N`, for a newtype `N` over `B` | `x` as an `N` | none; the domain is checked where the value is stored |
| `unwrap (x: N) -> B`, for a newtype `N` over `B` | the base value | none |
| `value (k: Key r.S)` | the key's value: the key field's value, or for a composite key the record of its fields | none |

```adt
-- given u: Key users, t: Key teams
type Cents = new I64
type Team = { id: Key teams, name: Text }
type Membership = { user: Key users, team: Key teams, role: Text }
relation teams : Team key id generate sequence
relation memberships : Membership key (user, team)
unwrap(Cents(250))                      -- 250 : I64
value(memberships.key(user: u, team: t))  -- { user: u, team: t }
```

`r.key(v)` builds a key from its value ([model.md](model.md#81-reference-types)).

Rulings: types/int-overflow/trap, types/fault-forms/checked, types/decimal/scale-typed, types/num-mix/explicit

## 4. Text

| Function | Result | Faults |
| --- | --- | --- |
| `a ++ b` with `a, b: Text` | the concatenation, normalized to NFC | none |
| `a ++ b` with `a, b: Bytes` | the concatenation | none |
| `bytes_len (t: Text) -> I64`, `bytes_len (b: Bytes) -> I64` | the number of bytes (UTF-8 bytes for text) | none |
| `scalar_len (t: Text) -> I64` | the number of Unicode scalar values | none |
| `grapheme_len (t: Text) -> I64` | the number of extended grapheme clusters, by the recorded Unicode version | none |
| `starts_with (t: Text, p: Text) -> Bool` | `t` begins with the scalar sequence `p` | none |
| `ends_with (t: Text, p: Text) -> Bool` | `t` ends with `p` | none |
| `contains (t: Text, p: Text) -> Bool` | `p` occurs in `t`; `true` when `p` is empty | none |
| `matches (t: Text, re: constant Text) -> Bool` | `re` matches somewhere in `t` | none; an invalid `re` is a compile error |
| `lower (t: Text) -> Text` | Unicode full default lowercase mapping, normalized to NFC | none |
| `upper (t: Text) -> Text` | Unicode full default uppercase mapping, normalized to NFC | none |
| `fold (t: Text) -> Text` | Unicode full case folding, normalized to NFC | none |
| `collate (t: Text, c: constant Text) -> SortKey` | the UTS #10 sort key of `t` under collation `c` | none; an unknown or unversioned `c` is a compile error |
| `text (x) -> Text` | the textual form of a number, a time value, a `Uuid` (§2.7) or a newtype over `Text` | none |
| `utf8 (b: Bytes) -> Option Text` | `b` decoded and normalized to NFC; `None` when `b` is not UTF-8 | none |
| `bytes (t: Text) -> Bytes` | the UTF-8 encoding of `t` | none |

Comparison of text is by scalar sequence on NFC text. None of these
functions depends on a locale.

`matches` uses the RE2 regular expression syntax, without
backreferences or lookaround. It matches over Unicode scalar values;
`.` and character classes match scalar values, and Unicode classes
(`\p{L}`) follow the recorded Unicode version. A match may occur
anywhere unless the pattern is anchored with `^` (start of text) or `$`
(end of text). The pattern is case-sensitive unless it begins with
`(?i)`, which compares by simple case folding. Matching takes time
linear in the length of `t`.

`text(x)` gives, for an integer, its decimal digits with a leading `-`
when negative; for a `Decimal s`, the digits with exactly `s`
fractional digits; for a float, the shortest decimal that reads back
as the same value, or `NaN`, `inf`, `-inf`; for a time value, the form
of [interop.md](interop.md#1-textual-value-syntax). A collation name
`c` carries its version, `"de-u-ks-level2@icu-74"`; the collations an
engine ships are listed in [catalog.md](catalog.md).

```adt
from i in identities.EmailPass
where ends_with(lower(i.email), "@example.com") and scalar_len(i.email) <= 254
select { id: key i, label: "user " ++ text(scalar_len(i.email)) }
```

Rulings: types/text-length/explicit, types/text-eq/nfc, types/collation/sortkey-fn, types/collation/version/versioned-name

## 5. Bytes

`Bytes` has `++`, `bytes_len` and comparison by unsigned byte order
(§4). `utf8` and `bytes` convert between `Bytes` and `Text`.

Rulings: model/bytes

## 6. Time

### 6.1 Building lengths

| Function | Result | Faults |
| --- | --- | --- |
| `micros`, `millis`, `seconds`, `minutes`, `hours` `(n: I64) -> Duration` | `n` of the unit | `ArithmeticOverflow` |
| `days`, `weeks`, `months`, `years` `(n: I64) -> Period` | `days(n)`, `days(7 * n)`, `months(n)`, `months(12 * n)` | `OutOfRange` when a count leaves the `I32` range |

A `DURATION` literal (`30s`, `5min`) is a `Duration`
([syntax.md](syntax.md#25-literals)).

### 6.2 Arithmetic on time

| Expression | Signature | Rule |
| --- | --- | --- |
| `a + b`, `a - b` | `Duration -> Duration -> Duration` | exact |
| `-d` | `Duration -> Duration` | exact |
| `d * n` | `Duration -> I64 -> Duration` | exact |
| `d / n` | `Duration -> I64 -> Duration` | truncating toward zero; faults `DivisionByZero` |
| `p + q`, `p - q` | `Period -> Period -> Period` | month and day counts separately |
| `-p` | `Period -> Period` | both counts negated |
| `p * n` | `Period -> I64 -> Period` | both counts multiplied |
| `i + d`, `i - d` | `Instant -> Duration -> Instant` | exact |
| `a - b` | `Instant -> Instant -> Duration` | exact |
| `dt + d`, `dt - d` | `DateTime -> Duration -> DateTime` | civil, 86400-second days |
| `a - b` | `DateTime -> DateTime -> Duration` | civil |
| `a - b` | `Date -> Date -> Duration` | a whole number of days |
| `z + d`, `z - d` | `ZonedDateTime -> Duration -> ZonedDateTime` | exact on the instant, then shown in `z`'s zone |

Every operation faults with `ArithmeticOverflow` when its result leaves its
type's range. A `Period` does not add to an `Instant`, `Date` or
`DateTime` by `+`: calendar arithmetic is `add_period`.

### 6.3 Calendar arithmetic

| Function | Result |
| --- | --- |
| `add_period (x: Date, p: Period, o: constant Overflow)` | `Date` for `Clamp`, `Option Date` for `Reject` |
| `add_period (x: DateTime, p: Period, o: constant Overflow)` | `DateTime` for `Clamp`, `Option DateTime` for `Reject` |
| `add_period (x: ZonedDateTime, p: Period, o: constant Overflow)` | `ZonedDateTime` for `Clamp`, `Option ZonedDateTime` for `Reject` |

`add_period` adds the months first, then the days. When the month
arithmetic reaches a day the month does not have (January 31 plus one
month), `Clamp` moves to the last day of the month and `Reject` gives
`None`. On a `ZonedDateTime` the wall time is kept in the zone, and a
wall time that does not exist on the resulting day is resolved as in
§6.4. Faults: `ArithmeticOverflow` when the result leaves the type's range.

```adt
-- given d: Date
add_period(d, months(1), Clamp)     -- 2021-01-31 gives 2021-02-28
add_period(d, months(1), Reject)    -- 2021-01-31 gives None
```

### 6.4 Zones

| Function | Result | Faults |
| --- | --- | --- |
| `zoned (dt: DateTime, zone: Text, o: constant Overflow)` | `ZonedDateTime` for `Clamp`, `Option ZonedDateTime` for `Reject` | `UnknownZone` |
| `in_zone (i: Instant, zone: Text) -> ZonedDateTime` | the civil time of `i` in `zone` | `UnknownZone` |
| `to_instant (z: ZonedDateTime) -> Instant` | the instant `z` names | none |

A civil time inside a gap of the zone's rules (a spring-forward
transition) does not exist: `Clamp` moves it forward by the length of
the gap, and `Reject` gives `None`. A civil time inside an overlap (a
fall-back transition) names two instants, and `zoned` and `to_instant`
use the earlier one. `to_instant` of a stored value whose civil time
falls in a gap under the zone rules now in force resolves as `Clamp`
does. The zone rules in force are the current ones for every caller
([model.md](model.md#36-time)).

```adt
-- given i: Instant, dt: DateTime
in_zone(i, "Europe/Lisbon")
zoned(dt, "America/New_York", Reject)
```

Rulings: types/durations/split, types/calendar-overflow/explicit, types/zoned/zoned, types/tzdb/catalog, types/time-scale/smeared

## 7. Aggregates

Aggregates are calls whose argument is a generator form
`e for x in q [where p]`. Their range rules, the empty-range rules
and user-defined aggregates are in
[queries.md](queries.md#10-aggregates).

| Aggregate | Element type | Nonempty range | Possibly empty range |
| --- | --- | --- | --- |
| `count(e for ...)` | any | `I64` | `I64`; `0` when empty |
| `sum(e for ...)` | `N` | `N` | `N`; `0` when empty |
| `min(e for ...)`, `max(e for ...)` | `a: Ord` | `a` | `Option a` |
| `avg(e for ...)` | `Float` | the same type | `Option` of it |

Integer and decimal `sum` is the exact sum of its elements, checked
against the result type once, at the end: it faults with
`ArithmeticOverflow` only when that exact sum is out of range of the
result type (for `Decimal s`, more than 38 digits), in whatever order
the elements are added.

An aggregate ranges over any set expression with set semantics, except
that `sum` and `avg` refuse a generator whose source, after
normalisation, is a comprehension whose `select` is not
key-preserving. A `select` is key-preserving when it is exactly the
binder of a generator over a relation or a keyed set: a relation
variable, a constructor relation, a constructor set, a backlink or
another key-preserving comprehension. The rule is syntactic: a
generator over a `Set T` parameter is accepted.
`count(a for a in (from p in posts select p.author))` counts distinct
authors ([queries.md](queries.md#10-aggregates)).

Rulings: lang/empty-agg/nonempty, lang/collection/set-agg/over-binders

## 8. Built-in forms

These forms are part of the language rather than functions. They are
listed here so that the prelude names every built-in operation.

| Form | Defined in |
| --- | --- |
| `key e`, `r.key(v)`, `r.key(f: v, g: w)` | [model.md](model.md#81-reference-types) |
| `tag(e)` | [queries.md](queries.md#92-the-tag-of-a-sum) |
| `try e` | [queries.md](queries.md#154-try) |
| `fail t`, `t: Text` | [writes.md](writes.md#6-upsert) |
| `exists x in q where p` | [queries.md](queries.md) |
| `T.parse (t: Text) -> Option T`, `print (v: T) -> Text` | [interop.md](interop.md#11-one-syntax) |
| `inserted(R)`, `deleted(R)`, `prior(R)` | [constraints.md](constraints.md#6-transition-constraints) |
| `generate sequence`, `generate uuidv7`, `generate clock` | [model.md](model.md#73-generated-keys), [derived.md](derived.md#4-defaults) |
| `Group K T`, the type of a group binder `g`: `g.key : K`, and `g` is a nonempty `Set T` wherever one is expected | [queries.md](queries.md#91-group-into) |
| `unwrap(x)`, `value(k)` | §3.5 |

Rulings: lang/group/ctor-key/tag-type, types/fault-forms/try, types/id-type/key-type

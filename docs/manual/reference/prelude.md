# The prelude

The prelude is the module every file and every ad hoc text reads
without qualification. It holds the base types, a handful of sums such
as `Bool` and `Option`, and every built-in function. This page lists
them by group, with what each returns, the faults it can raise and an
example. The full rules are in [the prelude part of the
specification](../../spec/prelude.md) and, for the base types,
[model.md](../../spec/model.md#3-base-types).

A fault is an error raised while a statement runs, such as dividing by
zero. It aborts the statement, and the shell prints it as ``fault
`DivisionByZero` ``. The faults are the constructors of the prelude sum
`Fault`, listed [below](#fault).

## The examples on this page

The examples run against a small database. Save this as `demo.adt`:

```adt demo.adt
module demo

type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
relation pets : Pet key name

type Member = { user: Text, team: Text, role: Text }
relation members : Member key (user, team)

type Cents = new I64
type Email = new Text where matches(self, "^[^@]+@[^@]+$")

fn instant (t: Text) -> Set Instant = match Instant.parse(t) | Some s -> [s.value] | None -> []
fn date (t: Text) -> Set Date = match Date.parse(t) | Some s -> [s.value] | None -> []
fn datetime (t: Text) -> Set DateTime = match DateTime.parse(t) | Some s -> [s.value] | None -> []
```

Time values have no literal; you write them with `T.parse`, which
returns an `Option` because the text may not parse. The three
functions at the end turn that `Option` into a set of zero or one
values, so that `from i in instant("...")` binds `i` to the parsed
value. Create the database and add a few rows:

```console
$ adtb --create --schema demo.adt demo.db
$ adtb demo.db
adtb> insert pets.Cat { name: "tom", lives: 9 }
adtb> insert pets.Cat { name: "kit", lives: 3 }
adtb> insert pets.Dog { name: "rex", good: true }
adtb> insert members { user: "ann", team: "red", role: "lead" }
```

A query whose result is one value prints that value; a query whose
result is a set prints each element on its own line.

## Base types

| Type | Values | Literal | Prints as |
| --- | --- | --- | --- |
| `I8`, `I16`, `I32`, `I64` | signed integers of 8 to 64 bits | `42` | `42` |
| `Decimal s` | exact decimals with `s` fractional digits, `s` at most 38 | `10.50` | `10.50` |
| `F32`, `F64` | IEEE 754 floats | `1.5e0` | `1.5` |
| `Text` | Unicode text, kept in NFC | `"tom"` | `"tom"` |
| `Bytes` | byte strings | none | `0x00ff` |
| `Instant` | points in time, UTC, microsecond precision | none | `2026-10-25T09:30:00.000000Z` |
| `Date` | calendar dates | none | `2026-10-25` |
| `Time` | times of day | none | `09:30:00.000000` |
| `DateTime` | a date and a time, no zone | none | `2026-10-25T09:30:00.000000` |
| `ZonedDateTime` | a `DateTime` and an IANA zone name | none | `2026-10-25T09:30:00.000000[Europe/Paris]` |
| `Duration` | exact lengths of time, in microseconds | `90s`, `5min` | `90s` |
| `Period` | calendar lengths: months and days | none | `P1M15D` |
| `SortKey` | collation sort keys | none | `0x2d0105` |

Integers have no unsigned or wider types, and no number converts to
another type on its own: `i32(5) + i64(1)` is refused, and you convert
with a [conversion](#conversions). A numeric literal takes the type its
position needs; with nothing to go by, `42` is an `I64`, `10.50` a
`Decimal 2` and `1.5e0` an `F64`. The units of a `Duration` literal
are `us`, `ms`, `s`, `min`, `h` and `d`, where `d` is 86400 seconds.

A text literal takes the escapes `\"`, `\\`, `\n`, `\r`, `\t` and
`\u{...}`. Text is normalized to NFC wherever it is built, so `"e\u{301}"`
(an `e` and a combining accent) is the one character `é`. Text compares
by code point, with no locale: `"Z" < "a"`.

Floats never fault. `-0.0` is stored as `0.0`, every NaN is one value,
NaN equals itself and sorts above every other float, including
infinity.

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> 1.25 + 1.5
adtb> 1.25 * 1.5
adtb> 1 + 1.5
adtb> "a\tb\n\"c\""
adtb> "Z" < "a"
adtb> 1.0e0 / 0.0e0
adtb> (0.0e0 / 0.0e0) > (1.0e0 / 0.0e0)
adtb> -0.0e0
adtb> i32(5) + i64(1)
2.75
1.875
2.5
"a\tb\n\"c\""
True
inf
True
0.0
`+` does not apply to I32 and I64
```

The scale of a decimal is part of its type: `+` and `-` give the larger
of the two scales and `*` their sum, so `1.25 * 1.5` has three
fractional digits. A product whose scale would pass 38 is refused.

### Values with no literal

Every type `T` has `T.parse (t: Text) -> Option T`, which reads a value
in the form the shell prints it, and every value has
`print (v: T) -> Text`, which gives that form. `T.parse` returns `None`
when the text is not a value of `T`. A type that is not a single name
is parenthesised: `(Set I64).parse(...)`, `(Tag Pet).parse(...)`.

```console
$ adtb demo.db
adtb> Instant.parse("2026-10-25T09:30:00Z")
adtb> Instant.parse("2026-10-25T09:30:00.5Z")
adtb> Date.parse("2026-10-25")
adtb> Time.parse("09:30:00")
adtb> DateTime.parse("2026-10-25T09:30:00")
adtb> Period.parse("P1M-15D")
adtb> Duration.parse("90s")
adtb> Bytes.parse("0x00FF")
adtb> F64.parse("NaN")
adtb> I64.parse("12")
adtb> I64.parse("twelve")
adtb> Text.parse("\"hi\"")
adtb> (Set I64).parse("[3, 1]")
adtb> print(2.50)
adtb> print(pets.key("tom"))
adtb> print([3, 1, 2])
Some { value: 2026-10-25T09:30:00.000000Z }
Some { value: 2026-10-25T09:30:00.500000Z }
Some { value: 2026-10-25 }
Some { value: 09:30:00.000000 }
Some { value: 2026-10-25T09:30:00.000000 }
Some { value: P1M-15D }
Some { value: 90s }
Some { value: 0x00ff }
Some { value: NaN }
Some { value: 12 }
None
Some { value: "hi" }
Some { value: [1, 3] }
"2.50"
"pets.key(\"tom\")"
"[1, 2, 3]"
```

A time is printed with six fractional digits of a second; when reading
one you may write fewer, or none. A set prints its elements in
ascending order, each once.

## Prelude types

### Bool

```adt fragment
type Bool ordered = False | True
```

`true` and `false` are spellings of `True` and `False`, and
`False < True`. `a and b` evaluates `b` only when `a` is `True`, and
`a or b` only when `a` is `False`. `not a` is the other constructor.
`if c then a else b` is `match c | True -> a | False -> b`.

```console
$ adtb demo.db
adtb> true and false
adtb> not true
adtb> if 1 > 2 then "yes" else "no"
False
False
"no"
```

### Option and Result

```adt fragment
type Option a ordered = None | Some { value: a }
type Result a e = Ok { value: a } | Err { error: e }
```

`Option` is how adtb says a value may be absent; there is no NULL.
`None` sorts before every `Some`, and two `Some` values sort by their
`value`. `Result` has no order. You read the value with a `match`:

```console
$ adtb demo.db
adtb> match I64.parse("12") | Some n -> n.value + 1 | None -> 0
adtb> match I64.parse("x") | Some n -> n.value + 1 | None -> 0
13
0
```

A constructor such as `None` or `Ok { value: 1 }` written where nothing
fixes its type parameters is refused, for example ``the type argument
`e` of `Result` is not known here; give the expected type``. Written as
a field value, an argument or a match arm, it takes its type from
there.

### Tag

`Tag T`, for a sum `T`, is a sum with one nullary constructor for each
constructor of `T`, of the same names. `tag(e)` gives the constructor
of `e` as a `Tag T` value, which you can select, compare and group by.

```console
$ adtb demo.db
adtb> from p in pets select { name: p.name, kind: tag(p) } order by p.name
adtb> from p in pets group p by tag(p) into g select { kind: g.key, n: count(x for x in g) }
adtb> (Tag Pet).parse("Cat")
adtb> (Tag Pet).parse("Bird")
{ kind: Cat, name: "kit" }
{ kind: Dog, name: "rex" }
{ kind: Cat, name: "tom" }
{ kind: Cat, n: 2 }
{ kind: Dog, n: 1 }
Some { value: Cat }
None
```

### Rounding and Overflow

```adt fragment
type Rounding = HalfEven | HalfUp | HalfDown | Up | Down | Ceiling | Floor
type Overflow = Clamp | Reject
```

A function that may round takes a `Rounding`, and nothing rounds
without one.

| Rounding | Rounds to |
| --- | --- |
| `HalfEven` | the nearest value; a tie goes to the even last digit |
| `HalfUp` | the nearest value; a tie goes away from zero |
| `HalfDown` | the nearest value; a tie goes toward zero |
| `Up` | away from zero |
| `Down` | toward zero |
| `Ceiling` | toward positive infinity |
| `Floor` | toward negative infinity |

`Overflow` says what calendar and zone functions do with a date or
time that does not exist, such as January 31 plus one month: `Clamp`
moves to the nearest one that does, and `Reject` gives `None`. See
[Time](#time).

### Fault

```adt fragment
type Fault = DivisionByZero | ArithmeticOverflow | OutOfRange | NotFinite
           | NoSuchDate | UnknownZone | InvalidText | InvalidSpan
           | NegativeCount | NoSuchRow | FunctionFailed
```

| Fault | Raised by |
| --- | --- |
| `DivisionByZero` | `/`, `%` or `div` with a zero divisor |
| `ArithmeticOverflow` | an integer, decimal or time result outside its type's range |
| `OutOfRange` | a conversion whose argument is outside the target's range |
| `NotFinite` | converting NaN or an infinity to an integer or decimal |
| `NoSuchDate` | a calendar or zone function with `Clamp` that finds no valid time |
| `UnknownZone` | a zone name the database's zone rules do not define |
| `InvalidText` | text a function cannot read, such as `uuid("nope")` |
| `InvalidSpan` | `span(s, e)` with `s` after `e` |
| `NegativeCount` | `take n` or `top n` with a negative `n` |
| `NoSuchRow` | following a key along a path when it names no row |
| `FunctionFailed` | a user-defined function that fails |

`try e` catches a fault raised by `e` and gives `Ok { value: v }` or
`Err { error: f }`, a `Result` whose error is a `Fault`:

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> 7 / 0
adtb> try (7 / 0)
adtb> try (7 / 1)
fault `DivisionByZero`
Err { error: DivisionByZero }
Ok { value: 7 }
```

### Uuid

```adt fragment
type Uuid = new Bytes where bytes_len(self) == 16
```

| Function | Result | Faults |
| --- | --- | --- |
| `uuid (t: Text) -> Uuid` | the UUID written in the 8-4-4-4-12 hexadecimal form, either case | `InvalidText` |
| `text (u: Uuid) -> Text` | the canonical form, lowercase | none |

A key declared `generate uuidv7` holds a `Uuid`.

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> uuid("0190A4B2-7C3E-7D4F-9A21-5B6C7D8E9F01")
adtb> text(uuid("0190a4b2-7c3e-7d4f-9a21-5b6c7d8e9f01"))
adtb> uuid("nope")
uuid("0190a4b2-7c3e-7d4f-9a21-5b6c7d8e9f01")
"0190a4b2-7c3e-7d4f-9a21-5b6c7d8e9f01"
fault `InvalidText`
```

### Span

```adt fragment
type Span a ordered = { start: a, end: a }
```

A `Span a` is the half-open interval from `start`, included, to `end`,
excluded, over an ordered type `a`. You build one only with `span`.

| Function | Result | Faults |
| --- | --- | --- |
| `span (start: a, end: a) -> Span a` | the span | `InvalidSpan` when `start > end` |
| `start (s: Span a) -> a` | its start | none |
| `end (s: Span a) -> a` | its end | none |
| `overlaps (x: Span a, y: Span a) -> Bool` | whether the two share a point | none |
| `covers (x: Span a, y: Span a) -> Bool` | whether `y` lies inside `x` | none |
| `covers (x: Span a, t: a) -> Bool` | whether `t` lies inside `x` | none |

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> span(3, 7)
adtb> end(span(3, 7))
adtb> overlaps(span(1, 5), span(4, 9))
adtb> overlaps(span(1, 5), span(5, 9))
adtb> covers(span(1, 10), 10)
adtb> from i in instant("2026-10-25T09:00:00Z") select covers(span(i, i + 1h), i + 30min)
adtb> span(7, 3)
span(3, 7)
7
True
False
False
True
fault `InvalidSpan`
```

### Other prelude types

These sums and records appear in the values the engine reports rather
than in your own data:

| Type | What it is |
| --- | --- |
| `Error` and its reason sums | the errors a call can return ([catalog.md](../../spec/catalog.md#4-error-sums)) |
| `Principal` | the caller: `EndUser { name: Text } \| Service { name: Text } \| Anonymous`, unless the schema declares its own |
| `Position` | `{ epoch: I64, lsn: I64 }`, a commit position |
| `TxnId` | a newtype over `I64`, a transaction's id |
| `Report` | a newtype over `Bytes`, the rows of a violated constraint |
| `Origin`, `Outcome` | what an audited event ran and how it ended |
| `RejectedRow` | the row type of a relation that receives an insert's rejected rows |

## Arithmetic

### Operators

| Operator | Operands | Faults |
| --- | --- | --- |
| `a + b`, `a - b`, `a * b` | two integers of one type | `ArithmeticOverflow` |
| `a / b` | two integers of one type; truncates toward zero | `DivisionByZero`; `ArithmeticOverflow` for the minimum value divided by `-1` |
| `a % b` | two integers of one type; the result has the sign of `a` | `DivisionByZero` |
| `-a` | an integer | `ArithmeticOverflow` for the minimum value |
| `a + b`, `a - b` | two decimals; the result has the larger scale | `ArithmeticOverflow` above 38 digits |
| `a * b` | two decimals; the result scale is the sum of the scales | `ArithmeticOverflow` above 38 digits |
| `a + b`, `a - b`, `a * b`, `a / b`, `-a` | two floats of one type | none |

Decimals have no `/` and no `%`: divide them with `div`. Integer
arithmetic never wraps.

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> 1 + 2 * 3
adtb> -7 / 2
adtb> -7 % 2
adtb> 9223372036854775807 + 1
adtb> i8(100) + i8(100)
adtb> 1.0 / 3.0
7
-3
-1
fault `ArithmeticOverflow`
fault `ArithmeticOverflow`
decimals have no `/` and no `%`; use `div` with a scale and a rounding
```

### Checked arithmetic

Each checked function returns `None` where its operator would fault and
`Some { value: v }` otherwise, and never faults.

| Function | Arguments |
| --- | --- |
| `checked_add`, `checked_sub`, `checked_mul` | two integers or two decimals |
| `checked_div`, `checked_rem` | two integers |
| `checked_neg` | one integer |

```console
$ adtb demo.db
adtb> checked_add(9223372036854775807, 1)
adtb> checked_add(i8(100), i8(100))
adtb> checked_sub(1, 2)
adtb> checked_div(7, 0)
adtb> checked_rem(7, 2)
adtb> checked_neg(5)
adtb> checked_mul(1.25, 2.0)
None
None
Some { value: -1 }
None
Some { value: 1 }
Some { value: -5 }
Some { value: 2.500 }
```

### Decimal functions

| Function | Result | Faults |
| --- | --- | --- |
| `div (a: Decimal s, b: Decimal t, u, r: Rounding) -> Decimal u` | `a / b` rounded to scale `u` by `r` | `DivisionByZero`; `ArithmeticOverflow` |
| `round (a: Decimal s, u, r: Rounding) -> Decimal u` | `a` rounded to a smaller scale `u` by `r` | none |
| `rescale (a: Decimal s, u) -> Decimal u` | `a` at a scale `u` of at least `s`, exactly | `ArithmeticOverflow` above 38 digits |
| `abs (a: N) -> N` | the absolute value of any number | `ArithmeticOverflow` for the minimum integer |

The scale `u` is written as a literal, since it is part of the result
type.

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> div(10.00, 3.00, 2, HalfEven)
adtb> div(1, 3, 4, HalfUp)
adtb> round(2.345, 2, HalfEven)
adtb> round(2.345, 2, HalfUp)
adtb> round(-2.345, 1, Floor)
adtb> round(2.5, 0, HalfEven)
adtb> round(3.5, 0, HalfEven)
adtb> rescale(2.5, 3)
adtb> abs(-2.50)
adtb> div(1.00, 0.00, 2, HalfEven)
adtb> rescale(2.50, 1)
3.33
0.3333
2.34
2.35
-2.4
2
4
2.500
2.50
fault `DivisionByZero`
`rescale` takes a scale of at least the operand's 2; use `round` to narrow
```

### Conversions

No number converts to another type implicitly. Each conversion that
can fault also has a checked form, named `checked_` followed by its
name (`checked_i8`, `checked_decimal`, ...), which takes the same
arguments and returns an `Option`.

| Function | From | Result | Faults |
| --- | --- | --- | --- |
| `i8`, `i16`, `i32`, `i64` `(x)` | any integer | the integer | `OutOfRange` |
| `i8`, `i16`, `i32`, `i64` `(x, r: Rounding)` | a decimal or float | `x` rounded by `r` | `OutOfRange`; `NotFinite` |
| `decimal (x, s)` | any integer | `x` at scale `s` | `OutOfRange` above 38 digits |
| `decimal (x, s, r: Rounding)` | a float | `x` rounded to scale `s` by `r` | `OutOfRange`; `NotFinite` |
| `f32 (x)`, `f64 (x)` | an integer, a decimal or the other float type | the nearest float | `f32` only: `OutOfRange` |

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> i8(100)
adtb> i8(300)
adtb> checked_i8(300)
adtb> i64(2.5, HalfEven)
adtb> i32(2.7, Floor)
adtb> i64(2.5e0, HalfUp)
adtb> i64(0.0e0 / 0.0e0, Floor)
adtb> decimal(3, 2)
adtb> f64(2.50)
adtb> f32(16777217)
100
fault `OutOfRange`
None
2
2
3
fault `NotFinite`
3.00
2.5
16777216.0
```

### Newtypes and keys

| Function | Result | Faults |
| --- | --- | --- |
| `N (x: B) -> N`, for a newtype `N` over `B` | `x` as an `N` | none |
| `unwrap (x: N) -> B` | the underlying value | none |
| `r.key(v)`, `r.key(f: v, g: w)` | the key of relation `r` with key value `v`, or with key fields `f` and `g` | none |
| `value (k: Key r) -> K` | the key's value, or for a composite key the record of its fields | none |

A newtype's domain, the `where` of its declaration, is not checked when
you build a value: `Email("not an email")` is a value. It is checked
when the value is stored, as a constraint.

```console
$ adtb demo.db
adtb> Cents(250)
adtb> unwrap(Cents(250))
adtb> Email("not an email")
adtb> value(pets.key("tom"))
adtb> members.key(user: "ann", team: "red")
adtb> value(members.key(user: "ann", team: "red"))
Cents(250)
250
Email("not an email")
"tom"
members.key(team: "red", user: "ann")
{ team: "red", user: "ann" }
```

## Text

| Function | Result |
| --- | --- |
| `a ++ b` | the concatenation of two texts, or of two byte strings |
| `bytes_len (t: Text) -> I64` | the number of UTF-8 bytes |
| `scalar_len (t: Text) -> I64` | the number of Unicode scalar values |
| `grapheme_len (t: Text) -> I64` | the number of user-perceived characters (extended grapheme clusters) |
| `starts_with (t: Text, p: Text) -> Bool` | whether `t` begins with `p` |
| `ends_with (t: Text, p: Text) -> Bool` | whether `t` ends with `p` |
| `contains (t: Text, p: Text) -> Bool` | whether `p` occurs in `t`; `True` when `p` is empty |
| `matches (t: Text, re: Text) -> Bool` | whether the regular expression `re` matches somewhere in `t` |
| `lower (t: Text) -> Text` | the Unicode lowercase of `t` |
| `upper (t: Text) -> Text` | the Unicode uppercase of `t` |
| `fold (t: Text) -> Text` | the Unicode case folding of `t`, for comparing without case |
| `collate (t: Text, c: Text) -> SortKey` | the sort key of `t` under the collation `c` |
| `text (x) -> Text` | the textual form of a number, a time value, a `Uuid` or a newtype over `Text` |
| `utf8 (b: Bytes) -> Option Text` | `b` decoded as UTF-8, or `None` when it is not |
| `bytes (t: Text) -> Bytes` | the UTF-8 encoding of `t` |

None of these faults, and none depends on a locale. Text has no plain
"length": you choose bytes, scalar values or graphemes.

`matches` uses the RE2 syntax, without backreferences or lookaround.
It matches anywhere unless you anchor it with `^` and `$`. The flags
`i`, `m`, `s` and `U` are set with `(?i)` and the like. `\d`, `\s` and
`\w` are the Unicode classes, so `\d` matches any decimal digit.
The pattern must be a literal; an invalid one is refused before the
statement runs.

`collate` takes a collation name that includes its CLDR version, such
as `"de@cldr-49"` or `"en-u-ks-level2@cldr-49"`; ordering by its result
sorts text the way that locale does.

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> "caf" ++ "é"
adtb> bytes_len("café")
adtb> scalar_len("café")
adtb> scalar_len("🇫🇷")
adtb> grapheme_len("🇫🇷")
adtb> starts_with("hello", "he")
adtb> contains("hello", "")
adtb> matches("ann@example.com", "^[^@]+@[^@]+$")
adtb> matches("ABC", "(?i)abc")
adtb> matches("٣", "^\\d$")
adtb> upper("straße")
adtb> fold("Straße")
adtb> lower("ÀB")
adtb> text(2.50)
adtb> text(0.1e0 + 0.2e0)
adtb> text(90s)
adtb> from p in pets select p.name order by collate(p.name, "en@cldr-49")
adtb> matches("a", "(")
"café"
5
4
2
1
True
True
True
True
True
"STRASSE"
"strasse"
"àb"
"2.50"
"0.30000000000000004"
"90s"
"kit"
"rex"
"tom"
`(` is not a regular expression: regex parse error:
    (
    ^
error: unclosed group
```

## Bytes

`Bytes` has `++`, `bytes_len`, and comparison by unsigned byte order.
`utf8` and `bytes` convert between `Bytes` and `Text`. A byte string
prints as `0x` and two lowercase hex digits per byte; `0x` alone is
the empty one.

```console
$ adtb demo.db
adtb> bytes("hi")
adtb> bytes("hi") ++ bytes("!")
adtb> bytes_len(bytes("hé"))
adtb> utf8(bytes("hé"))
adtb> match Bytes.parse("0xff") | Some b -> utf8(b.value) | None -> None
0x6869
0x686921
3
Some { value: "hé" }
None
```

## Time

No expression reads a clock. A query that needs the current time takes
it as a parameter, and a write can record it with a generated default
(`default f = generate clock`).

### Building lengths

| Function | Result | Faults |
| --- | --- | --- |
| `micros`, `millis`, `seconds`, `minutes`, `hours` `(n: I64) -> Duration` | `n` of the unit | `ArithmeticOverflow` |
| `days`, `weeks`, `months`, `years` `(n: I64) -> Period` | a calendar length; a week is 7 days and a year 12 months | `OutOfRange` |

A `Duration` is exact and a `Period` is calendar: one month is not a
number of seconds. They are different types and neither converts to
the other. A duration prints in the largest unit that divides it
exactly.

```console
$ adtb demo.db
adtb> millis(1500)
adtb> minutes(90)
adtb> hours(48)
adtb> 1h + 30min
adtb> 2h * 3
adtb> 7d / 2
adtb> weeks(2)
adtb> years(2)
adtb> days(1) + months(1)
adtb> -months(1)
1500ms
90min
2d
90min
6h
84h
P0M14D
P24M0D
P1M1D
P-1M0D
```

### Arithmetic on time

| Expression | Result |
| --- | --- |
| `a + b`, `a - b`, `-d`, `d * n`, `d / n` on durations | a `Duration`; `d / n` truncates and faults `DivisionByZero` |
| `p + q`, `p - q`, `-p`, `p * n` on periods | a `Period`, months and days counted separately |
| `i + d`, `i - d`, with `i` an `Instant` and `d` a `Duration` | an `Instant` |
| `a - b` on two `Instant`s, two `DateTime`s or two `Date`s | the `Duration` between them |
| `dt + d`, `dt - d` on a `DateTime` | a `DateTime`, counting 86400-second days |
| `z + d`, `z - d` on a `ZonedDateTime` | the instant moved by `d`, shown in `z`'s zone |

Every one faults with `ArithmeticOverflow` when the result leaves its
type's range. A `Period` is never added with `+` to a point in time;
use `add_period`.

```console
$ adtb demo.db
adtb> from i in instant("2026-10-25T09:30:00Z") select i + 90min
adtb> from a in instant("2026-10-25T09:30:00Z"), b in instant("2026-10-24T09:00:00Z") select a - b
adtb> from a in date("2026-10-25"), b in date("2026-10-01") select a - b
adtb> from d in datetime("2026-10-25T09:30:00") select d + 1h
2026-10-25T11:00:00.000000Z
1470min
24d
2026-10-25T10:30:00.000000
```

### Calendar arithmetic

`add_period (x, p: Period, o: Overflow)` adds a period to a `Date`, a
`DateTime` or a `ZonedDateTime`: the months first, then the days. When
the months land on a day the month does not have, `Clamp` moves to the
last day of the month and gives the value, and `Reject` gives an
`Option` that is `None`. The `Overflow` is written in place, since it
decides the result type.

```console
$ adtb demo.db
adtb> from d in date("2021-01-31") select add_period(d, months(1), Clamp)
adtb> from d in date("2021-01-31") select add_period(d, months(1), Reject)
adtb> from d in datetime("2026-10-25T09:30:00") select add_period(d, years(1), Clamp)
2021-02-28
None
2027-10-25T09:30:00.000000
```

### Zones

| Function | Result | Faults |
| --- | --- | --- |
| `zoned (dt: DateTime, zone: Text, o: Overflow)` | the civil time `dt` in `zone`: a `ZonedDateTime` for `Clamp`, an `Option` of one for `Reject` | `UnknownZone`, `ArithmeticOverflow` |
| `in_zone (i: Instant, zone: Text) -> ZonedDateTime` | the civil time of `i` in `zone` | `UnknownZone`, `ArithmeticOverflow` |
| `to_instant (z: ZonedDateTime) -> Instant` | the instant `z` names | `ArithmeticOverflow` |

A zone is an IANA name, spelled exactly: `"Europe/Lisbon"`, never
`"europe/lisbon"`. A civil time that a spring-forward transition skips
does not exist: `Clamp` moves it forward by the length of the gap and
`Reject` gives `None`. A civil time that a fall-back transition repeats
names the earlier of its two instants. A `ZonedDateTime` keeps its zone
name and civil time; it has no order, since two zones' clocks do not
line up.

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> from d in datetime("2026-10-25T09:30:00") select zoned(d, "Europe/Paris", Clamp)
adtb> from d in datetime("2026-03-29T01:30:00") select zoned(d, "Europe/Lisbon", Clamp)
adtb> from d in datetime("2026-03-29T01:30:00") select zoned(d, "Europe/Lisbon", Reject)
adtb> from i in instant("2026-10-25T09:30:00Z") select in_zone(i, "Asia/Tokyo")
adtb> from i in instant("2026-10-25T09:30:00Z") select to_instant(in_zone(i, "Asia/Tokyo"))
adtb> from i in instant("2026-10-25T09:30:00Z") select in_zone(i, "europe/lisbon")
2026-10-25T09:30:00.000000[Europe/Paris]
2026-03-29T02:30:00.000000[Europe/Lisbon]
None
2026-10-25T18:30:00.000000[Asia/Tokyo]
2026-10-25T09:30:00.000000Z
fault `UnknownZone`
```

## Aggregates

An aggregate is a call whose argument is a generator,
`e for x in q`, optionally with `where p`. The full rules are in
[queries.md](../../spec/queries.md#10-aggregates).

| Aggregate | Elements | Result |
| --- | --- | --- |
| `count(e for ...)` | any | an `I64`; `0` when the range is empty |
| `sum(e for ...)` | numbers | the same type; `0` when empty |
| `min(e for ...)`, `max(e for ...)` | an ordered type | an `Option` of it; `None` when empty |
| `avg(e for ...)` | floats | an `Option` of the same type; `None` when empty |

A range is a set, so equal elements count once: `count` over the
values of a `select` counts distinct values. For that reason `sum` and
`avg` refuse a range that is a `select` of values, which may have
merged equal ones; range over the rows and aggregate the field
instead. An integer or decimal `sum` faults with `ArithmeticOverflow`
only when the exact total is out of range.

<!-- exit 1 -->
```console
$ adtb demo.db
adtb> count(p for p in pets)
adtb> count(p for p in pets where p.name == "zzz")
adtb> sum(c.lives for c in pets.Cat)
adtb> sum(c.lives for c in pets.Cat where c.lives > 100)
adtb> min(p.name for p in pets)
adtb> max(c.lives for c in pets.Cat where c.lives > 100)
adtb> avg(f64(c.lives) for c in pets.Cat)
adtb> count(x for x in [1, 1, 2])
adtb> sum(x for x in (from c in pets.Cat select c.lives))
3
0
12
0
Some { value: "kit" }
None
Some { value: 6.0 }
2
`sum` ranges over rows, not over values a `select` may have merged; range over the rows and aggregate the field
```

## Built-in forms

These are part of the language rather than functions, and are listed
here so that this page names every built-in operation.

| Form | Meaning |
| --- | --- |
| `key e` | the key of the row `e` |
| `r.key(v)`, `r.key(f: v, g: w)` | a key of relation `r` ([Newtypes and keys](#newtypes-and-keys)) |
| `tag(e)` | the constructor of `e` as a `Tag` value ([Tag](#tag)) |
| `try e` | `e`'s fault as a `Result` ([Fault](#fault)) |
| `fail t` | end a write with the error text `t` |
| `exists x in q where p` | whether `q` has an element satisfying `p` |
| `T.parse(t)`, `print(v)` | read and print the textual form ([Values with no literal](#values-with-no-literal)) |
| `inserted(r)`, `deleted(r)`, `prior(r)` | the rows a statement inserted or deleted, and the state before it, in a transition constraint ([constraints.md](../../spec/constraints.md#6-transition-constraints)) |
| `generate sequence`, `generate uuidv7`, `generate clock` | generated keys and defaults ([model.md](../../spec/model.md#73-generated-keys)) |
| `g.key` for a group `g` | the value a `group ... by ... into g` grouped by ([queries.md](../../spec/queries.md#91-group-into)) |
| `unwrap(x)`, `value(k)` | the value under a newtype or a key |

## Not yet available

`caller`, the principal running a statement, is refused in this engine
with `` `caller` in a statement does not run in this engine yet ``.

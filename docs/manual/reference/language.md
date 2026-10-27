# The language

This page lists the forms of adtb's language, one construct at a time:
what you write, what it means in a sentence, and an example where one
helps. Each entry links to the specification for the full rule. The
grammar is in [syntax.md](../../spec/syntax.md); the meaning of
queries is in [queries.md](../../spec/queries.md) and of writes in
[writes.md](../../spec/writes.md).

The examples run against this schema, saved as `library.adt`:

```adt library.adt
module library

type Isbn = new Text where scalar_len(self) == 13
type Format ordered = Paperback | Hardback | Ebook { size: I64 }

type Book = { isbn: Isbn, title: Text, year: I32, format: Format }
relation books : Book key isbn

type Member = { id: Key members, name: Text, email: Text }
relation members : Member key id generate sequence
  unique (email)

type Loan { id: Key loans, book: Key books, member: Key members on delete cascade }
  = Out { renewals: I64 }
  | Returned { late: Bool }
  | Lost
relation loans : Loan key id generate sequence
  index (member)
  default Out.renewals = 0

set Closed of loans = Returned | Lost

fn status (l: Loan) -> Text = match l
  | Out _ -> "out"
  | Returned _ -> "returned"
  | Lost _ -> "lost"

check printed on books (year >= 1450)

constraint one_out =
  from b in books
  where count(l for l in loans.Out where l.book == key b) > 1
  select b
```

```console
$ adtb --create --schema library.adt library.db -c 'insert books { isbn: Isbn("9780141439518"), title: "Emma", year: 1815, format: Paperback }
insert books { isbn: Isbn("9780199535569"), title: "Persuasion", year: 1817, format: Ebook { size: 3 } }
insert books { isbn: Isbn("9780199536559"), title: "Mansfield Park", year: 1814, format: Hardback }
insert members { name: "ann", email: "ann@example.org" }
insert members { name: "bo", email: "bo@example.org" }'
```

## Files and modules

There are three kinds of text:

- A schema file is `module NAME` followed by declarations. It describes
  a whole schema, and the shell accepts one as a database's first
  migration.
- A migration file is `module NAME` followed by schema operators. It is
  one change to a database's schema.
- Ad hoc text is what you send to a database: queries, writes,
  `transact` blocks and commands. It has no module header.

Names in a file resolve in its own module first, then in the prelude.
A name from another module is qualified with `::`, as in
`library::books`. In ad hoc text an unqualified name means the
declaration of that name in a user module, or in the prelude when no
user module declares it.
See [syntax.md §3](../../spec/syntax.md#3-files).

## Lexical basics

`--` starts a comment that runs to the end of the line. Line breaks and
indentation carry no meaning in the language itself; the shell uses
them to tell where one text ends ([the shell](shell.md#scripts)).

Names starting with a lowercase letter name values, fields, binders,
relations, functions, constraints and modules. Names starting with an
uppercase letter name types, constructors and named sets. Identifiers
are ASCII. A `(` directly after a name, with no space, starts a call:
`count(...)` calls, while `fn status (l: Loan)` declares.

| Literal | Type | Example |
| --- | --- | --- |
| integer | `I64`, or the integer type the context expects | `42`, `007` |
| decimal | `Decimal s`, with `s` the digits after the point unless the context gives a scale | `2.50` |
| float | `F64`, or `F32` when expected | `1.5e3` |
| text | `Text`, in double quotes, with escapes `\"`, `\\`, `\n`, `\r`, `\t`, `\u{e9}` | `"Emma"` |
| duration | `Duration`, with unit `us`, `ms`, `s`, `min`, `h` or `d` | `90s`, `5min` |
| boolean | the constructors `True` and `False` of `Bool` | `true`, `false` |
| set | `Set T`; equal elements are one, order means nothing | `[1, 2, 3]` |

There is no negative literal: `-3` is negation applied to `3`. A value
with no literal is written as `T.parse` of its text, which returns an
`Option`, since the text may not parse:

```console
$ adtb library.db
adtb> -3 + 007
4
adtb> 1.5e3
1500.0
adtb> "caf\u{e9}"
"café"
adtb> 5min
5min
adtb> [3, 1, 2, 1]
1
2
3
adtb> Date.parse("2026-10-25")
Some { value: 2026-10-25 }
adtb> F64.parse("inf")
Some { value: inf }
adtb> \q
```

See [syntax.md §2](../../spec/syntax.md#2-lexical-structure).

## Types

### Base types

| Type | Values |
| --- | --- |
| `I8`, `I16`, `I32`, `I64` | signed integers; overflow is a fault, never a wrap |
| `Decimal s` | exact decimals with `s` digits after the point |
| `F32`, `F64` | IEEE floats |
| `Text` | Unicode text |
| `Bytes` | byte strings |
| `Instant` | points in time, in microseconds, UTC |
| `Date`, `Time`, `DateTime` | civil dates and times with no zone |
| `ZonedDateTime` | a civil date and time in a named zone |
| `Duration` | exact lengths of time |
| `Period` | calendar lengths in months and days |

No numeric type converts to another implicitly: `f64(x)`, `i64(x)` and
the other conversions are prelude functions. See
[model.md §3](../../spec/model.md#3-base-types) and
[the prelude](prelude.md).

### Records and sums

`type Book = { ... }` declares a record: a type with one constructor,
named after the type. `type Format = Paperback | Hardback | Ebook {
size: I64 }` declares a sum: a value of `Format` is exactly one of its
constructors. A constructor without fields, like `Paperback`, is
nullary; a sum whose constructors are all nullary is an enumeration.

A record before the `=` is a header: fields every constructor has. In
`Loan` above every loan has `id`, `book` and `member`, and only an
`Out` loan has `renewals`. `ordered` after the type name gives the type
an order: by constructor in declaration order, then by fields. Without
it a sum cannot be compared with `<` or used to order a result.

See [model.md §2](../../spec/model.md#2-sums).

### Newtypes

`type Isbn = new Text` declares a distinct type over a base type.
`Isbn("...")` builds one and `unwrap(i)` gives the text back. A `where`
clause makes it a domain: the condition, with `self` the underlying
value, is checked wherever a value is stored.

<!-- exit 1 -->
```console
$ adtb library.db
adtb> insert books { isbn: Isbn("123"), title: "Short", year: 2000, format: Hardback }
constraint `library::Isbn.domain` is violated by 1 row
adtb> from b in books where b.title == "Emma" select unwrap(b.isbn)
"9780141439518"
adtb> \q
```

See [model.md §4](../../spec/model.md#4-newtypes).

### Type parameters and library sums

A type may take parameters, written in lowercase after its name:
`type Box a = Box { item: a }`, used as `Box Text`. The prelude
declares the parametric sums you will use most:

```text
type Option a ordered = None | Some { value: a }
type Result a e = Ok { value: a } | Err { error: e }
```

`Option T` is how a value may be absent; there is no NULL. See
[model.md §5](../../spec/model.md#5-type-parameters).

### Key types

`Key books` is a reference to a row of `books`: the type of a field
that holds another row's key. `Key loans.Out` and `Key loans.(Returned
| Lost)` narrow it to some constructors, and `Key loans.Closed` to a
named set. A key is built with `r.key(v)`, and `value(k)` reads the
key's value back. A key field typed by its own relation's key, like
`id: Key members` in `members`, holds the row's own key. See
[model.md §8](../../spec/model.md#8-references).

### Row types, tags and sets

`Loan.Out` is the type of a row of constructor `Out`, and
`Loan.(Returned | Lost)` of a row of either. `Tag Loan` is a sum of
nullary constructors with the same names as `Loan`'s, the value
`tag(l)` returns. `Set T` is a set of `T`, the type of every query.
`T -> U` is a function type, allowed only for function parameters.

## Declarations

### Type

```text
type NAME [params] [ordered] [header] = constructors | record | new BASE [where e]
```

See [syntax.md §5.1](../../spec/syntax.md#51-types).

### Relation

```text
relation NAME : TYPE key FIELD [generate sequence | generate uuidv7]
  [unique (FIELDS) [in CTORS]]
  [index (FIELDS) [in CTORS]]
  [default [CTOR.]FIELD = EXPR]
  [computed FIELD: TYPE = EXPR]
```

A relation holds a set of rows of one type, identified by its key.

- `key f` names the key field; `key (a, b)` is a composite key.
  `generate sequence` and `generate uuidv7` fill an omitted key. A
  generated key is unique, not consecutive.
- `unique (f)` refuses two rows with equal `f`; `in C` limits it to
  constructor `C` or a set of constructors.
- `index (f)` speeds up lookups by `f`; it changes no answer.
- `default f = e` gives the value an insert takes for `f` when it
  omits the field; `default C.f = e` only for constructor `C`.
  `default f = generate clock` gives the time of the statement.
- `computed f: T = e` adds a field computed from the row, with `self`
  the row.

<!-- exit 1 -->
```console
$ adtb library.db
adtb> insert members { name: "cy", email: "ann@example.org" }
constraint `library::members.unique(email) in Member` is violated by 2 rows
adtb> insert loans.Out from (from m in members, b in books where m.name == "ann" and b.title == "Emma" select { member: key m, book: key b })
adtb> from l in loans.Out select l.renewals
0
adtb> \q
```

See [syntax.md §5.2](../../spec/syntax.md#52-relation-variables) and
[model.md §7](../../spec/model.md#7-keys).

### Named constructor set

`set Closed of loans = Returned | Lost` names some constructors of one
relation. It is used as `loans.Closed`, as `in Closed` in that
relation's clauses, and as a pattern. See
[syntax.md §5.3](../../spec/syntax.md#53-named-constructor-sets).

### Function

`fn NAME (p: T, ...) -> U = e` declares a function. The parameter and
result types are required, and a function without parameters, `fn
NAME () -> Set T = ...`, is a view. A parameter may have a function
type, and a call may pass a lambda for it. See
[syntax.md §5.4](../../spec/syntax.md#54-functions).

`fn NAME (p: T, ...) -> U = host "NAME" version "V" [deterministic]`
declares a function whose body a program supplies when it opens the
database, under that name and version. The shell supplies none, so a
query that calls one there fails with
`` fatal: host function `NAME` is missing ``. A program supplies the
body as [the Rust API](rust-api.md#host-functions) shows.

### Check and constraint

`check NAME on R (e)` requires `e` of every row of `R`, with the row's
fields in scope by name. `constraint NAME = q` requires query `q` to
return nothing: every row it returns is a violation. Both hold after
every statement, and a write that breaks one is refused. The engine
admits a constraint only if it can check it without reading a whole
relation for every change. See
[constraints.md](../../spec/constraints.md).

<!-- exit 1 -->
```console
$ adtb library.db
adtb> insert books { isbn: Isbn("9780000000001"), title: "Old", year: 1200, format: Hardback }
constraint `library::printed` is violated by 1 row
adtb> insert loans.Out from (from m in members, b in books where m.name == "bo" and b.title == "Emma" select { member: key m, book: key b })
constraint `library::one_out` is violated by 1 row
adtb> \q
```

## Expressions

### Precedence

From loosest to tightest:

| Forms | Associativity |
| --- | --- |
| `from`, `match`, `fix`, `fn`, `exists`, `if` | extend as far right as they can |
| `union` | left |
| `or` | left |
| `and` | left |
| `not` | prefix |
| `==` `!=` `<` `<=` `>` `>=` | none: `a < b < c` is refused |
| `+` `-` `++` | left |
| `*` `/` `%` | left |
| unary `-`, `key`, `try` | prefix |
| `.f`, `.C`, `.<r.f`, calls | left |

An open form such as `match` or `from` in any position but the last
is parenthesised. `++` concatenates text. `/` on integers truncates,
and dividing by zero is a fault. See
[syntax.md §6.1](../../spec/syntax.md#61-precedence).

### Names, fields and paths

| Form | Meaning |
| --- | --- |
| `b.title` | field `title` of `b` |
| `l.book.title` | follows the reference in `l.book` to the book, then reads its title |
| `m.<loans.member` | backlink: the loans whose `member` is `m` |
| `key b` | the key of row `b` |
| `books.key(Isbn("..."))` | the key of `books` with that value |
| `loans.Out`, `loans.(Returned \| Lost)`, `loans.Closed` | the rows of some constructors |

On a whole sum only header fields can be read; to read a constructor's
own fields, range over that constructor or `match`.
See [queries.md §7](../../spec/queries.md#7-navigation).

### Constructors and records

`Ebook { size: 3 }` builds a constructor value, and `Loan.Out { ... }`
qualifies it when the bare name is ambiguous. `{ who: "ann", n: 2 }` is
an anonymous record. `Book { b with year: 1816 }` is `b` with one field
replaced; for a record type the name may be left out, as
`{ b with year: 1816 }`. Every field is named; there is no shorthand.
See [syntax.md §6.3](../../spec/syntax.md#63-constructors-and-records).

### If, try and lambdas

`if c then a else b` chooses by a `Bool`. `try e` turns a fault into a
value: `Ok { value: v }`, or `Err { error: f }` with `f` a `Fault`.
`fn (x: T) -> e` is a lambda; it can be passed to a function that takes
a function, and is normalised away before the query runs.

```console
$ adtb library.db
adtb> if count(b for b in books) > 2 then "many" else "few"
"many"
adtb> try (10 / 0)
Err { error: DivisionByZero }
adtb> try (10 / 2)
Ok { value: 5 }
adtb> \q
```

See [queries.md §15](../../spec/queries.md#15-faults).

## Comprehensions

```text
from x in R, y in S where COND select EXPR order by KEYS take N after CURSOR
```

`from` binds names to the elements of sets, `where` keeps the bindings
whose condition holds, and `select` gives each result. The result is a
set: equal results are one. `from` and `where` may repeat in any order
before `select`.

```console
$ adtb library.db
adtb> from b in books where b.year < 1816 select b.title order by b.title
"Emma"
"Mansfield Park"
adtb> from m in members, l in loans where l.member == key m select { who: m.name, what: l.book.title }
{ what: "Emma", who: "ann" }
adtb> from m in members select { who: m.name, n: count(l for l in m.<loans.member) } order by m.name
{ n: 1, who: "ann" }
{ n: 0, who: "bo" }
adtb> from b in books group b by b.format into g select { format: g.key, n: count(x for x in g) } order by g.key
{ format: Paperback, n: 1 }
{ format: Hardback, n: 1 }
{ format: Ebook { size: 3 }, n: 1 }
adtb> \q
```

- A query in a `select` is a nested result, a set inside each row.
- `group e by k into g` partitions the bindings by `k`. After `into`
  only `g` is in scope: `g.key` is the value of `k`, and `g` is the
  set of the values of `e`. See
  [queries.md §9](../../spec/queries.md#9-grouping).
- `order by k` and `order by (k1 desc, k2)` present the outermost
  result in an order; `asc` is the default. An ordering key reads only
  the selected value: `select b.title order by b.year` is refused,
  while `select b order by b.year` and `select { t: b.title, y: b.year }
  order by b.year` are accepted.
- `take n` keeps the first `n`; `after c` starts after the cursor `c`,
  an `Option` of the ordering value. Both need an order that has no
  ties, so the selected value is a row or a key and the ordering
  includes its key or a unique field. There is no offset.

See [queries.md §3](../../spec/queries.md#3-comprehensions) and
[§14](../../spec/queries.md#14-ordering-top-n-and-pagination).

## Match and patterns

```text
match e | PATTERN [if GUARD] -> EXPR | ...
```

A `match` takes the first arm whose pattern matches and whose guard
holds. It must cover every constructor of the scrutinee's type by name;
leaving one out is an error. A wildcard `_` or a bare binder covers
scalars such as integers and text. In ad hoc text `_` may also cover
the constructors an arm does not name; in schema functions,
constraints and bundles it may not.

| Pattern | Matches |
| --- | --- |
| `Out o` | constructor `Out`, binding its row or value to `o` |
| `Out _` | constructor `Out`, binding nothing |
| `Paperback` | a nullary constructor of a value-level sum |
| `Ebook { size }` | `Ebook`, binding field `size` by its own name |
| `Returned { late: true }` | `Returned` whose `late` matches the pattern `true` |
| `Closed c` | any constructor of named set `Closed` |
| `Paperback \| Hardback` | either pattern |
| `3`, `"a"`, `-1` | an equal literal, for scalars |
| `x`, `_` | anything, for scalars |

A constructor of a relation-level sum always has its header fields, so
even `Lost` is matched as `Lost _`.

<!-- exit 1 -->
```console
$ adtb library.db
adtb> from b in books select match b.format | Paperback | Hardback -> "paper" | Ebook e if e.size > 2 -> "large file" | Ebook _ -> "small file"
"large file"
"paper"
adtb> from l in loans select match l | Closed _ -> "closed" | Out { renewals } -> "out, renewed " ++ print(renewals)
"out, renewed 0"
adtb> from l in loans select match l | Out _ -> 1 | Returned _ -> 2
match does not cover `Lost _`
adtb> match 3 | 1 -> "one" | 3 -> "three" | _ -> "other"
"three"
adtb> \q
```

See [queries.md §5](../../spec/queries.md#5-match) and
[§6](../../spec/queries.md#6-patterns).

## Other query forms

| Form | Meaning |
| --- | --- |
| `exists x in q where p` | whether some binding satisfies `p`; `not exists` for none |
| `q1 union q2` | the union of two sets; a comprehension operand is parenthesised |
| `count(e for x in q where p)` | an aggregate over the bindings: also `sum`, `min`, `max`, `avg` |
| `top n by k (q)` | the first `n` elements of comprehension `q` by `k`, at any depth |
| `fix r = q in e` | the least set `r` with `r = q`, where `q` may read `r` |
| `tag(e)` | the constructor of `e`, as a `Tag` value |

`count` counts bindings, `sum` of an empty range is `0`, and `min`,
`max` and `avg` return an `Option`, `None` over an empty range, unless
the range is a group. `avg` takes floats only, so convert first.

```console
$ adtb library.db
adtb> from b in books where not exists l in loans where l.book == key b select b.title order by b.title
"Mansfield Park"
"Persuasion"
adtb> (from b in books select b.title) union ["Sanditon"]
"Emma"
"Mansfield Park"
"Persuasion"
"Sanditon"
adtb> sum(b.year for b in books)
5446
adtb> avg(f64(b.year) for b in books)
Some { value: 1815.3333333333333 }
adtb> top 1 by (b.year desc, b.isbn) (from b in books select b)
Book { isbn: Isbn("9780199535569"), title: "Persuasion", year: 1817, format: Ebook { size: 3 } }
adtb> fix r = [1] union (from x in r where x < 4 select x + 1) in from x in r select x * 10
10
20
30
40
adtb> \q
```

See [queries.md §10](../../spec/queries.md#10-aggregates) and
[§11](../../spec/queries.md#11-recursion).

## Writes

A write computes a change set from one snapshot of the database and
applies it as a whole; queries never change anything. See
[writes.md](../../spec/writes.md).

| Form | Meaning |
| --- | --- |
| `insert r.C { ... }` | insert one row of constructor `C`; for a one-constructor relation, `insert r { ... }` |
| `insert r from (q)` | insert every value `q` returns; constructor values route to their constructors |
| `insert ... unless conflict on key else W` | an upsert: on a key conflict run `W`, with `old` the existing row and `new` the proposed one |
| `insert ... unless conflict on (f) else W` | the same, for a conflict on a unique |
| `update x in r where p set e` | replace each matching row by the whole new value `e` |
| `update x in r set C { x with ... }` | a retag when `C` is another constructor: the row keeps its key |
| `delete x in r where p` | delete each matching row |
| `match e \| P -> W ...`, `if c then W1 else W2` | choose a write |
| `(W1, W2)` | combine writes into one change set |
| `skip` | the empty write |
| `fail "message"` | refuse the statement with a message |

A key never changes. An `update` gives the whole new value, so every
field is in it, usually through `with`.

```console
$ adtb library.db
adtb> update b in books where b.title == "Emma" set Book { b with year: 1816 }
adtb> update l in loans.Out set Returned { l with late: false }
adtb> from l in loans select status(l)
"returned"
adtb> insert books { isbn: Isbn("9780141439518"), title: "Emma (2nd)", year: 1816, format: Paperback } unless conflict on key else update old set Book { old with title: new.title }
adtb> from b in books where b.year == 1816 select b.title
"Emma (2nd)"
adtb> insert members { name: "cy", email: "ann@example.org" } unless conflict on (email) else skip
adtb> delete m in members where m.name == "bo"
adtb> from m in members select m.name order by m.name
"ann"
adtb> \q
```

### Transact blocks

```text
transact
  let x = W_OR_EXPR
  W
  return e
```

A `transact` block runs its items as one statement: every item reads
the database as it was when the block began, the writes apply together
at the end, and if any item fails, faults or violates a constraint,
nothing commits. `let x = insert ...` binds the row the
insert made; `key x` is its key. `return e` gives the block's result.

At the prompt a block runs at the blank line after it:

<!-- exit 1 -->
```console
$ adtb library.db
adtb> transact
  ...   let m = insert members { name: "dee", email: "dee@example.org" }
  ...   let l = insert loans.Out { member: key m, book: books.key(Isbn("9780199535569")) }
  ...   return l.renewals
  ...
0
adtb> transact
  ...   insert members { name: "eve", email: "eve@example.org" }
  ...   fail "changed my mind"
  ...
changed my mind
adtb> from m in members select m.name order by m.name
"ann"
"dee"
adtb> \q
```

See [transactions.md §3](../../spec/transactions.md#3-transact-blocks).

## Schema operators

A migration file holds operators. Each is a verb, an object and
clauses. These run in the engine:

| Operator | Effect |
| --- | --- |
| `add module m`, `drop module m` | create or drop an empty module |
| `add type ...`, `drop type T`, `rename type T to U` | declare, drop or rename a type |
| `add relation ...`, `drop relation r`, `rename relation r to s` | declare, drop (when empty) or rename a relation |
| `add set S of r = ...`, `drop set S of r`, `rename set S of r to U` | named constructor sets |
| `add constructor T.C { ... } [before D \| after D] [arm f: P -> e]` | add a constructor, declared but not yet open for writes |
| `enable r.C`, `deprecate r.C`, `disable r.C` | open a constructor to writes, mark an enabled one deprecated, or return an enabled one to declared while it has no rows |
| `rename constructor T.C to D` | rename a constructor |
| `remove constructor T.C` | remove a deprecated constructor that has no rows |
| `merge constructor T.(C \| D) into E { ... } [by match ...]` | merge constructors into one, with a field whose enumeration names them; `by match` gives the field's value when it is not the constructor's name |
| `split constructor T.C into D { ... } \| E { ... } by match ...` | route the rows of one constructor onto several |
| `nest constructor T.(C \| D) into E { f: S }`, `unnest constructor T.E` | move constructors into a value-level sum field and back |
| `add field T.C.f: U fill e`, `drop field T.C.f`, `rename field T.C.f to g` | change a constructor's fields; `T.f` is a header field |
| `move field T.C.f to T fill match ...`, `move field T.f to T.(C \| D)` | move a field into the header, with values for the other constructors, or out to some constructors |
| `alter field T.f on delete ...` | change a reference's delete action |
| `add unique (f) in r.C`, `drop unique ...`, `add index ...`, `drop index ...` | uniques and indexes |
| `rekey relation r key (...)` | change a relation's key |
| `define fn ...`, `redefine fn ...`, `drop fn f` | functions |
| `add check ...`, `drop check c`, `add constraint ...`, `drop constraint c` | checks and constraints |
| `revert N` | undo migration `N` of the log, by its position |

`fill` gives existing rows a value for a new field; it is not a default
for later inserts. The `arm` clauses give the new constructor's arm of
each schema function that matches on the type. A schema file's
`default` clauses are applied, but the operators `add default`,
`add computed` and `script` are not yet available: the engine refuses
them with `this operator has no resolved form yet`.

See [syntax.md §8](../../spec/syntax.md#8-schema-operators),
[evolution.md §4](../../spec/evolution.md#4-operators) and
[Evolving a schema](../guides/evolving-a-schema.md).

## Ad hoc text

Ad hoc text is a sequence of items: queries, writes, `transact` blocks
and commands. Every item starts with a keyword or `(`, except a query
that comes first. The items of one text run in order as one
transaction, and the first that fails ends the text. A free lowercase
name is a parameter, which the shell cannot supply
([the shell](shell.md#parameters)). See
[queries.md §16](../../spec/queries.md#16-ad-hoc-statements).

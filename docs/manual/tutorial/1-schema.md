# 1. Declare a schema

Over these seven pages you build the database of a small animal
shelter: its pets, the people who adopt them, and the walks volunteers
take the dogs on. This page declares the first relations, creates the
database and puts rows in it. It assumes you have the `adtb` shell
installed, as [Getting started](../getting-started.md#build-the-shell)
shows.

## The schema file

A schema file names its module on its first line and then declares
types and relations. Save this as `shelter.adt`:

```adt shelter.adt
module shelter

-- The people who adopt pets, identified by email address.
type Adopter = { email: Text, name: Text, phone: Text }
relation adopters : Adopter key email

type Pet = { name: Text, species: Text, age: I64 }
relation pets : Pet key name

-- A kennel is identified by its building and its number in it.
type Kennel = { building: Text, number: I64, places: I64 }
relation kennels : Kennel key (building, number)

-- Donations have no natural identity, so the engine numbers them.
type Donation = { id: Key donations, donor: Text, amount: Decimal 2 }
relation donations : Donation key id generate sequence
```

A line comment starts with `--`. Every name you declare lives in the
module, here `shelter`.

`type Adopter = { ... }` declares a record type: a shape with named,
typed fields. Every field is required; there is no NULL, and a value
of `Adopter` always has all three fields. Type names start with an
uppercase letter, and relation and field names with a lowercase one.

`relation adopters : Adopter key email` declares a relation: a named,
stored set of `Adopter` values. The type and the relation are
separate declarations. The type says what a value looks like, and the
relation says where values of it are kept and how they are
identified.

## Keys

Every relation declares exactly one key, the fields that identify a
row. No two rows of a relation agree on their key, and there is no
hidden row id besides it. The key can be:

- one field, as `email` for adopters and `name` for pets;
- several fields, written in parentheses, as `(building, number)` for
  kennels;
- a field of type `Key r` for the relation itself, as `id: Key
  donations`, which `generate sequence` fills for you when an insert
  leaves it out. `Key r` is the type of a key of relation `r`;
  [page 5](5-references-and-constraints.md) covers it.

The full rules for keys are in [the specification](../../spec/model.md#7-keys).

## Create the database and insert rows

`--create` makes the database file from the schema; `--schema` names
the schema file. The shell then opens a prompt on the new database,
where you type texts. Each text runs as soon as it is complete, and
`\q` leaves:

```console
$ adtb --create --schema shelter.adt shelter.db
adtb> insert adopters { email: "ann@example.com", name: "Ann", phone: "555-0100" }
adtb> insert adopters { email: "bo@example.com", name: "Bo", phone: "555-0101" }
adtb> insert pets { name: "tom", species: "cat", age: 3 }
adtb> insert pets { name: "rex", species: "dog", age: 5 }
adtb> \q
```

This leaves `shelter.db` and a log directory, `shelter.db-log`, beside
it. An insert gives the whole value: every field, in any order. A
write prints nothing when it succeeds.

The shell reads the schema only when it creates the database. To
change the schema of a database that holds data you write a migration,
which [page 7](7-migrations.md) covers. Until then, while you are
still designing, the simplest way to try a changed `shelter.adt` is to
delete `shelter.db` and its log directory `shelter.db-log` and create
the database again.

## Read rows

Later runs open the database without `--create`:

```console
$ adtb shelter.db
adtb> from a in adopters select a order by a.email
Adopter { email: "ann@example.com", name: "Ann", phone: "555-0100" }
Adopter { email: "bo@example.com", name: "Bo", phone: "555-0101" }
adtb> from p in pets where p.age > 4 select p.name
"rex"
adtb> \q
```

A query reads `from` a relation, keeps the rows `where` its condition holds and
returns what `select` says. The shell prints each result on its own
line in the syntax you write values in, except a value of a type with
no literal (see [Base types](#base-types)). That prints as the text
its `T.parse` reads: a date prints as `2026-10-25`, which typed back
is integer subtraction, so write `Date.parse("2026-10-25")`. [Page 3](3-queries.md) covers
queries in depth.

The result of a query is a set, so it has no order of its own: `order
by` asks for one. Use it whenever you want the rows in a predictable
order.

## What the engine refuses

The engine checks every text against the schema before it runs it,
and checks every write against the keys:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> insert adopters { email: "ann@example.com", name: "Annie", phone: "555-0199" }
`shelter::adopters` already holds a row with key `adopters.key("ann@example.com")`
adtb> insert pets { name: "kit", species: "cat" }
field `age` of `Pet` is missing, and `pets` has no default for it
adtb> insert pets { name: "kit", species: "cat", age: "one" }
expected I64, found Text
adtb> from p in pets select p.colour
Pet has no field `colour`
adtb> \q
```

An insert never replaces a row: a second adopter with Ann's email is
refused, and Ann's row is unchanged. A refused text changes nothing.
An insert must give every field: the "no default" in the second
message refers to defaults, values a relation can declare for a field
an insert leaves out. This schema declares none;
[Evolving a schema](../guides/evolving-a-schema.md#defaults-and-computed-fields)
shows how to add one.

## Composite and generated keys

A composite key is built with the names of its fields. `kennels.key(building: "A", number: 1)`
is the key of kennel 1 in building A, and `key k` is the key of a row
`k`:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> insert kennels { building: "A", number: 1, places: 2 }
adtb> insert kennels { building: "A", number: 2, places: 1 }
adtb> insert kennels { building: "B", number: 1, places: 4 }
adtb> from k in kennels where key k == kennels.key(building: "A", number: 2) select k.places
1
adtb> insert donations { donor: "Ann", amount: 25.00 }
adtb> insert donations { donor: "Bo", amount: 10 }
adtb> from d in donations select { donor: d.donor, amount: d.amount } order by d.donor
{ amount: 25.00, donor: "Ann" }
{ amount: 10.00, donor: "Bo" }
adtb> insert donations { donor: "Cy", amount: 2.505 }
a literal with 3 fractional digits does not check against Decimal 2
adtb> \q
```

The donations got an `id` each without you giving one. A generated
key is distinct from every other key the relation has held, but it is
not a count: the numbers can have gaps and need not follow the order
of inserts, so do not read meaning into them.

A record you build in a `select`, such as `{ donor: ..., amount: ... }`,
is an anonymous record, and the shell prints its fields in name order.

## Base types

The types a field can have are built from these base types:

| Type | Values | Written as |
| --- | --- | --- |
| `I64` (also `I32`, `I16`, `I8`) | signed integers | `42`, `-7` |
| `Decimal s` | exact decimals with `s` digits after the point | `25.00`, `10` |
| `F64`, `F32` | floating point | `1.5e0`, `2.0e3` |
| `Text` | Unicode text | `"ann"`, `"line\n"` |
| `Bytes` | byte strings | no literal |
| `Duration` | exact lengths of time | `90s`, `15min`, `2h` |
| `Instant`, `Date`, `Time`, `DateTime` | points and civil times | no literal |

A type with no literal is read from text with a call such as
`Date.parse("2026-10-25")`, which returns an `Option`, since the text
may not be a date; the tutorial sticks to types with literals.

`Bool` is not a base type but a sum the prelude declares, with the
constructors `False` and `True`; you write them `false` and `true`,
and the shell prints them `False` and `True`. Arithmetic never wraps:
a result out of range, or a division by zero, is a fault that stops
the statement. The full list, with every literal form, is in
[the prelude reference](../reference/prelude.md) and
[the specification](../../spec/model.md#3-base-types).

## Next

`species: Text` lets any text in, so nothing stops a pet whose
species is `"cta"`, and a field that only cats have would have to be
given for dogs too. [Page 2](2-sums.md) replaces it with a sum.

# Getting started

This page takes you from nothing to a small database you can query,
change and migrate, using the `adtb` command-line shell. It assumes
you know what a database is, and nothing about adtb.

adtb stores relations whose rows can take one of several shapes. A
pet is a cat or a dog; both have a name, but only a cat counts its
lives. In adtb that is one relation, `pets`, of one type, `Pet`, with
two constructors, `Cat` and `Dog`. The engine knows which constructor
each row has, checks every query against it, and has no NULL: a field
a row does not have is a field its constructor does not declare.

## Build the shell

From a checkout of the repository, install the shell with Cargo:

```sh
cargo install --path crates/adtb-shell
```

This puts `adtb` on your `PATH`. `adtb --help` lists its flags.

## Write a schema

A schema file starts with the name of its module and then declares
types and relations. Save this as `shelter.adt`:

```adt shelter.adt
module shelter

type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
relation pets : Pet key name
```

`Pet { name: Text }` gives every pet a `name`. After the `=` come the
constructors, separated by `|`, each with the fields only it has.
`relation pets : Pet key name` declares a relation holding `Pet`
values, identified by `name`: no two pets share one. Integers are
`I64` and strings are `Text`.

## Create the database

```console
$ adtb --create --schema shelter.adt shelter.db
adtb> \q
```

This creates `shelter.db` and a log directory, `shelter.db-log`, beside
it, and opens the shell's prompt on the new database; `\q` leaves. The schema file becomes the database's first migration. If the
schema is refused, the shell says why and leaves no file behind.

## Insert rows

`-c` runs one text and exits. An insert into a relation of several
constructors names the constructor:

<!-- exit 0 0 1 -->
```console
$ adtb shelter.db -c 'insert pets.Cat { name: "tom", lives: 9 }'
$ adtb shelter.db -c 'insert pets.Dog { name: "rex", good: true }'
$ adtb shelter.db -c 'insert pets { name: "kit", lives: 3 }'
a single row is inserted into one constructor of Pet: name it, or insert constructor values with `from (...)`
```

The third insert is refused because it does not say whether `kit` is
a cat or a dog. A write prints nothing when it succeeds.

## Query

A query is a comprehension: `from` binds a name to each row, `where`
filters, `select` says what to return. The shell prints one result per
line, in the same syntax you write values in:

```console
$ adtb shelter.db -c 'from p in pets select p order by p.name'
Dog { name: "rex", good: True }
Cat { name: "tom", lives: 9 }
$ adtb shelter.db -c 'from c in pets.Cat where c.lives > 5 select c.name'
"tom"
```

`pets.Cat` ranges over the cats only, so `c.lives` is always there.
Over `pets` itself only the fields every pet has, here `name`, can be
read directly.

## Match on constructors

To read the fields of each constructor, `match` on the row. A match
names every constructor; leaving one out is an error, not a silent
gap:

<!-- exit 0 1 -->
```console
$ adtb shelter.db -c 'from p in pets select { name: p.name, kind: match p
  | Cat c -> "cat"
  | Dog d -> "dog" } order by p.name'
{ kind: "dog", name: "rex" }
{ kind: "cat", name: "tom" }
$ adtb shelter.db -c 'from p in pets select match p | Cat c -> c.name'
match does not cover `Dog _`
```

A record prints its fields in name order.

## Add a relation with a reference

A schema changes only by migration. A migration file starts with its
module, like a schema file, and then lists operators: `add` before a
declaration adds it. Save this as `adoptions.adt`:

```adt adoptions.adt
module shelter
add type Adoption = { pet: Key pets, by: Text }
add relation adoptions : Adoption key pet
```

`Key pets` is a reference: a field holding the key of a row of `pets`.
The engine keeps it valid, so an adoption always names a pet that
exists.

The database's log is its list of migrations, in order. To add one,
give the shell the whole list, one `--schema` per file; it applies the
ones the database has not seen yet:

<!-- exit 0 1 1 -->
```console
$ adtb --schema shelter.adt --schema adoptions.adt shelter.db -c 'insert adoptions { pet: pets.key("tom"), by: "ann" }'
$ adtb shelter.db -c 'insert adoptions { pet: pets.key("felix"), by: "bo" }'
constraint `shelter::adoptions.Adoption.pet` is violated by 1 row
$ adtb shelter.db -c 'delete p in pets where p.name == "tom"'
constraint `shelter::adoptions.Adoption.pet` is violated by 1 row
```

`pets.key("tom")` is the key of the pet named `tom`. There is no pet
`felix`, so the second insert is refused, and `tom` cannot be deleted
while an adoption names him. Once a migration is applied, later runs
need no `--schema`.

## Work in a transaction

Every `-c` text runs as one transaction. To run several texts as one,
start the shell without `-c` and use `\begin`. Each text runs when it
is complete; `\commit` keeps what the transaction did and `\rollback`
discards it. `\help` lists the commands and `\q` quits.

```console
$ adtb shelter.db
adtb> \begin
adtb> insert pets.Dog { name: "fido", good: false }
adtb> insert adoptions { pet: pets.key("fido"), by: "bo" }
adtb> from a in adoptions select a.by order by a.by
"ann"
"bo"
adtb> \rollback
adtb> from a in adoptions select a.by
"ann"
adtb> \q
```

Inside the transaction the second insert sees `fido`; after the
rollback neither row exists. A query or write that fails inside a
transaction prints its error and leaves the transaction open.

## Add a field

Cats should record their age. Adding a field to a constructor that
already has rows needs a value for those rows, given by `fill`. Save
this as `age.adt`:

```adt age.adt
module shelter
add field Pet.Cat.age: I64 fill 0
```

```console
$ adtb --schema shelter.adt --schema adoptions.adt --schema age.adt shelter.db -c 'from c in pets.Cat select c'
Cat { name: "tom", lives: 9, age: 0 }
$ adtb shelter.db -c 'insert pets.Cat { name: "kit", lives: 3, age: 1 }'
$ adtb shelter.db -c 'from c in pets.Cat select { name: c.name, age: c.age } order by c.name'
{ age: 1, name: "kit" }
{ age: 0, name: "tom" }
```

`fill 0` sets the age of every cat that existed before the migration.
It is not a default: a new cat states its age like any other field.

## Where next

The shell runs any ad hoc text the engine accepts: queries, writes
(`insert`, `update`, `delete`) and `transact` blocks that run several
writes as one. A file of texts can be piped to `adtb FILE` on standard
input; blank lines separate texts and lines starting with `--` are
comments. The normative rules for every construct are in
[the specification](../spec/README.md).

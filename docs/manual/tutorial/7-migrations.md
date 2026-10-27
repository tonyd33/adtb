# 7. Migrations

Until now, a changed schema meant a new database. Once the shelter
runs on adtb, its data has to survive every change, and the schema
changes only by migration: a file of operators, each a precise change
such as "add this field" or "rename that one", that the engine checks
against the data and applies as one step.

## The schema

This is the schema of [page 5](5-references-and-constraints.md) with
the `species` function of [page 4](4-match.md) added. Save it as
`shelter.adt`:

```adt shelter.adt
module shelter

type Adopter = { email: Text, name: Text, phone: Option Text }
relation adopters : Adopter key email
  index (phone)

type Size ordered = Small | Medium | Large
type Diet = Normal | Special { food: Text }

type Pet { name: Text, age: I64, size: Size, diet: Diet }
  = Cat { lives: I64, indoor: Bool }
  | Dog { breed: Text }
  | Rabbit
relation pets : Pet key name

type Adoption = { pet: Key pets, adopter: Key adopters, fee: Decimal 2 }
relation adoptions : Adoption key pet

type Walk = { id: Key walks, dog: Key pets.Dog on delete cascade, walker: Key adopters, minutes: I64 }
relation walks : Walk key id generate sequence

check sensible_age on pets (age >= 0 and age < 40)

constraint one_adopter_per_phone =
  from a in adopters, b in adopters
  where a.phone != None and a.phone == b.phone and key a != key b
  select a

fn species (p: Pet) -> Text = match p
  | Cat _ -> "cat"
  | Dog _ -> "dog"
  | Rabbit _ -> "rabbit"
```

```console
$ adtb --create --schema shelter.adt shelter.db
adtb> insert pets.Cat { name: "tom", age: 3, size: Small, diet: Normal, lives: 9, indoor: true }
adtb> insert pets.Cat { name: "kit", age: 1, size: Small, diet: Special { food: "kitten" }, lives: 9, indoor: false }
adtb> insert pets.Dog { name: "rex", age: 5, size: Large, diet: Normal, breed: "collie" }
adtb> insert pets.Rabbit { name: "bun", age: 1, size: Small, diet: Normal }
adtb> insert adopters { email: "ann@example.com", name: "Ann", phone: Some { value: "555-0100" } }
adtb> insert adopters { email: "bo@example.com", name: "Bo", phone: None }
adtb> \q
```

## The log

A database keeps its log: the list of migrations applied to it, in
order. The schema is what the log adds up to. Your schema file became
migration 1 when you created the database. The log is a relation you
can query, `catalog::migrations`:

```console
$ adtb shelter.db
adtb> from m in catalog::migrations select m.version
migrations.key(1)
adtb> \q
```

## Add a field

Cats should record whether they are vaccinated. A cat that is already
in the database needs a value for the new field, which `fill` gives.
Save this migration as `vaccinated.adt`:

```adt vaccinated.adt
module shelter
add field Pet.Cat.vaccinated: Bool fill false
```

`Pet.Cat.vaccinated` names the field by its type, its constructor and
its name; `Pet.vaccinated` would add a header field to every
constructor. To apply a migration, give the shell the whole log, one
`--schema` per file in order. It applies the files the database has
not seen yet:

```console
$ adtb --schema shelter.adt --schema vaccinated.adt shelter.db
adtb> from c in pets.Cat select c order by c.name
Cat { name: "kit", age: 1, size: Small, diet: Special { food: "kitten" }, lives: 9, indoor: False, vaccinated: False }
Cat { name: "tom", age: 3, size: Small, diet: Normal, lives: 9, indoor: True, vaccinated: False }
adtb> insert pets.Cat { name: "tigger", age: 2, size: Medium, diet: Normal, lives: 9, indoor: true }
field `vaccinated` of `Cat` is missing, and `pets` has no default for it
adtb> insert pets.Cat { name: "tigger", age: 2, size: Medium, diet: Normal, lives: 9, indoor: true, vaccinated: true }
adtb> \q
```

`fill false` set the field of the two cats that existed. It is not a
default: a new cat states it like any other field. A fill can also
compute each row's value with a match over the row: `fill match | Cat c
-> c.age > 2`.

Once a migration is applied it is in the database's log, and a later
run needs no `--schema` at all. When you do give the list, it must
agree with the log: the files the database has seen must be the same,
in the same order.

## Rename a field

```adt mobile.adt
module shelter
rename field Adopter.phone to mobile
```

```console
$ adtb --schema shelter.adt --schema vaccinated.adt --schema mobile.adt shelter.db
adtb> from a in adopters select { name: a.name, mobile: a.mobile } order by a.name
{ mobile: Some { value: "555-0100" }, name: "Ann" }
{ mobile: None, name: "Bo" }
adtb> from a in adopters select a.phone
Adopter has no field `phone`
adtb> update a in adopters where a.name == "Bo" set Adopter { a with mobile: Some { value: "555-0100" } }
constraint `shelter::one_adopter_per_phone` is violated by 2 rows
adtb> \q
```

The rename reached everything that named the field: the index and
the constraint now read `mobile`, and the constraint still holds.
Queries you typed with the old name, kept in your application, are the
part the engine cannot rewrite for you.

## Add a constructor

The shelter starts taking birds. Adding a constructor to `Pet` affects
every match on a pet, and `species` matches on one, so the migration
has to say what `species` returns for a bird. Without that, it is
refused:

```adt bird.adt
module shelter
add constructor Pet.Bird { sings: Bool }
```

```console
$ adtb --schema shelter.adt --schema vaccinated.adt --schema mobile.adt --schema bird.adt shelter.db
shelter.db: migration 4 is refused: match does not cover `Bird _`
```

A refused migration changes nothing, and the database stays at
migration 3. `arm species: Bird _ -> "bird"` gives the function its
new arm. Replace `bird.adt` with:

```adt bird.adt
module shelter
add constructor Pet.Bird { sings: Bool }
  arm species: Bird _ -> "bird"
```

```console
$ adtb --schema shelter.adt --schema vaccinated.adt --schema mobile.adt --schema bird.adt shelter.db
adtb> insert pets.Bird { name: "tweety", age: 1, size: Small, diet: Normal, sings: true }
constructor `pets.Bird` is Declared, not open for writes
adtb> \q
```

A new constructor starts declared: readable, but closed to inserts.
Every reader learns about birds first, and only then may anyone write
one. In a deployed system that is the window in which you upgrade
every program that reads pets, before any of them meets a bird. A
second migration opens it:

```adt enable-bird.adt
module shelter
enable pets.Bird
```

```console
$ adtb --schema shelter.adt --schema vaccinated.adt --schema mobile.adt --schema bird.adt --schema enable-bird.adt shelter.db
adtb> insert pets.Bird { name: "tweety", age: 1, size: Small, diet: Normal, sings: true }
adtb> from p in pets select { name: p.name, species: species(p) } order by p.name
{ name: "bun", species: "rabbit" }
{ name: "kit", species: "cat" }
{ name: "rex", species: "dog" }
{ name: "tigger", species: "cat" }
{ name: "tom", species: "cat" }
{ name: "tweety", species: "bird" }
adtb> \q
```

The reverse path is `deprecate pets.Bird`, which closes the
constructor to new rows while the old ones stay readable, and then,
once no rows are left, `remove constructor Pet.Bird`.
[Evolving a schema](../guides/evolving-a-schema.md) walks through it.

## A migration the data refuses

The engine checks a migration against the data before applying it. A
check that existing rows break is refused, and the refusal shows them:

```adt grown.adt
module shelter
add check grown on pets (age >= 2)
```

```console
$ adtb --schema shelter.adt --schema vaccinated.adt --schema mobile.adt --schema bird.adt --schema enable-bird.adt --schema grown.adt shelter.db
shelter.db: migration 6 is refused: constraint shelter::grown is violated by Cat { name: "kit", age: 1, size: Small, diet: Special { food: "kitten" }, lives: 9, indoor: False, vaccinated: False }, Rabbit { name: "bun", age: 1, size: Small, diet: Normal }, Bird { name: "tweety", age: 1, size: Small, diet: Normal, sings: True }
```

## Revert

Every operator has an inverse, and `revert` applies the inverse of an
earlier migration as a new one. Reverting the rename gives the field
its old name back:

```adt unrename.adt
module shelter
revert 0003
```

```console
$ adtb --schema shelter.adt --schema vaccinated.adt --schema mobile.adt --schema bird.adt --schema enable-bird.adt --schema unrename.adt shelter.db
adtb> from a in adopters select { name: a.name, phone: a.phone } order by a.name
{ name: "Ann", phone: Some { value: "555-0100" } }
{ name: "Bo", phone: None }
adtb> from m in catalog::migrations where value(m.version) > 1 select { version: value(m.version), text: m.text } order by value(m.version)
{ text: "module shelter\nadd field Pet.Cat.vaccinated: Bool fill false\n", version: 2 }
{ text: "module shelter\nrename field Adopter.phone to mobile\n", version: 3 }
{ text: "module shelter\nadd constructor Pet.Bird { sings: Bool }\n  arm species: Bird _ -> \"bird\"\n", version: 4 }
{ text: "module shelter\nenable pets.Bird\n", version: 5 }
{ text: "module shelter\nrevert 0003\n", version: 6 }
adtb> \q
```

A migration is named by its four-digit number, `0003`. The log only
grows: a revert does not remove migration 3, it adds migration 6, which
undoes it. The refused `grown.adt` never entered the log, so
`unrename.adt` is migration 6.

## The log must agree

Since the log is the history, a file the database has applied cannot
change afterwards. Give the shell a different migration 2 and it
refuses to open the database:

```adt vaccinated-2.adt
module shelter
add field Pet.Cat.vaccinated: Bool fill true
```

```console
$ adtb --schema shelter.adt --schema vaccinated-2.adt shelter.db
shelter.db: the file's migration 2 differs from the log's
```

Fix a mistake with a new migration, never by editing an old one.
[The migration log](../concepts/migration-log.md) explains the model
behind this, and [Evolving a schema](../guides/evolving-a-schema.md)
covers the other operators: moving fields between constructors,
splitting and merging constructors, and what each refuses. The full
list is in [the specification](../../spec/evolution.md#4-operators).

Not yet available: the specification's `script` operator, which runs
a write as part of a migration, and `add default`, which gives a field
a default for later inserts. The engine refuses both with "this
operator has no resolved form yet".

This is the end of the tutorial. [The language
reference](../reference/language.md) lists every form you have met,
and the [guides](../README.md#guides) take each topic further.

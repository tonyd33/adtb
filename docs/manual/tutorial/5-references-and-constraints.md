# 5. References and constraints

On [page 3](3-queries.md) an adoption held a pet's name and an
adopter's email as plain text, so nothing stopped it naming a pet that
does not exist. This page makes those fields references, which the
engine keeps valid, and adds rules of your own: a check on each row
and a constraint over the whole database.

## The schema

Save this as `shelter.adt`:

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

-- Volunteers walk the dogs, and only the dogs.
type Walk = { id: Key walks, dog: Key pets.Dog on delete cascade, walker: Key adopters, minutes: I64 }
relation walks : Walk key id generate sequence

check sensible_age on pets (age >= 0 and age < 40)

constraint one_adopter_per_phone =
  from a in adopters, b in adopters
  where a.phone != None and a.phone == b.phone and key a != key b
  select a
```

`Key pets` is the type of a key of `pets`: a reference to a pet. An
adoption's `pet` field holds the key of the pet adopted, and since
`pet` is also the key of `adoptions`, a pet is adopted at most once.
`Key pets.Dog` is narrower: a reference to a pet that is a dog. The
`index (phone)` clause on `adopters` is explained with the constraint
it serves, below.

Create the database with the pets and adopters of the earlier pages:

```console
$ adtb --create --schema shelter.adt shelter.db
adtb> insert pets.Cat { name: "tom", age: 3, size: Small, diet: Normal, lives: 9, indoor: true }
adtb> insert pets.Cat { name: "kit", age: 1, size: Small, diet: Special { food: "kitten" }, lives: 9, indoor: false }
adtb> insert pets.Dog { name: "rex", age: 5, size: Large, diet: Normal, breed: "collie" }
adtb> insert pets.Dog { name: "fido", age: 2, size: Medium, diet: Special { food: "low fat" }, breed: "pug" }
adtb> insert pets.Rabbit { name: "bun", age: 1, size: Small, diet: Normal }
adtb> insert adopters { email: "ann@example.com", name: "Ann", phone: Some { value: "555-0100" } }
adtb> insert adopters { email: "bo@example.com", name: "Bo", phone: None }
adtb> insert adopters { email: "cy@example.com", name: "Cy", phone: None }
adtb> \q
```

## Write and follow references

`pets.key("tom")` builds the key of the pet named `tom`. A key is its
own type, so the text `"kit"` is not a `Key pets`, and a reference to
a pet that does not exist is refused:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> insert adoptions { pet: pets.key("tom"), adopter: adopters.key("ann@example.com"), fee: 40.00 }
adtb> insert adoptions { pet: pets.key("rex"), adopter: adopters.key("ann@example.com"), fee: 60.00 }
adtb> insert adoptions { pet: pets.key("bun"), adopter: adopters.key("bo@example.com"), fee: 15.00 }
adtb> insert adoptions { pet: pets.key("nemo"), adopter: adopters.key("bo@example.com"), fee: 5.00 }
constraint `shelter::adoptions.Adoption.pet` is violated by 1 row
adtb> insert adoptions { pet: "kit", adopter: adopters.key("bo@example.com"), fee: 5.00 }
expected Key pets, found Text
adtb> from a in adoptions select a order by key a
Adoption { pet: pets.key("bun"), adopter: adopters.key("bo@example.com"), fee: 15.00 }
Adoption { pet: pets.key("rex"), adopter: adopters.key("ann@example.com"), fee: 60.00 }
Adoption { pet: pets.key("tom"), adopter: adopters.key("ann@example.com"), fee: 40.00 }
adtb> \q
```

Every reference is a foreign key, and the engine names it after the
field that holds it: `shelter::adoptions.Adoption.pet`.

A dot follows a reference to the row it names, so `a.pet.age` is the
age of the adopted pet. `d.<adoptions.adopter` goes the other way, a
backlink: the adoptions whose `adopter` field refers to `d`. The joins
of page 3 become paths:

```console
$ adtb shelter.db
adtb> from a in adoptions select { pet: a.pet.name, adopter: a.adopter.name, age: a.pet.age } order by a.pet.name
{ adopter: "Bo", age: 1, pet: "bun" }
{ adopter: "Ann", age: 5, pet: "rex" }
{ adopter: "Ann", age: 3, pet: "tom" }
adtb> from d in adopters select { name: d.name, pets: from a in d.<adoptions.adopter select a.pet.name } order by d.name
{ name: "Ann", pets: ["rex", "tom"] }
{ name: "Bo", pets: ["bun"] }
{ name: "Cy", pets: [] }
adtb> from p in pets where not exists a in p.<adoptions.pet select p.name
"fido"
"kit"
adtb> \q
```

## References to some constructors

A walk's `dog` is a `Key pets.Dog`, so a walk can only name a dog.
`pets.key("rex")` is a `Key pets`, which could be any pet, so it does
not fit. The key of a row you read from `pets.Dog` does: `key d`, with
`d` ranging over the dogs, is a `Key pets.Dog`. You insert walks from a
query that reads them:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> insert walks { dog: pets.key("rex"), walker: adopters.key("bo@example.com"), minutes: 30 }
expected Key pets.Dog, found Key pets
adtb> insert walks from (from d in pets.Dog where d.name == "rex" select { dog: key d, walker: adopters.key("bo@example.com"), minutes: 30 })
adtb> insert walks from (from d in pets.Dog select { dog: key d, walker: adopters.key("ann@example.com"), minutes: 20 })
adtb> from w in walks select { dog: w.dog.name, breed: w.dog.breed, walker: w.walker.name, minutes: w.minutes } order by (w.dog.name, w.walker.name)
{ breed: "pug", dog: "fido", minutes: 20, walker: "Ann" }
{ breed: "collie", dog: "rex", minutes: 20, walker: "Ann" }
{ breed: "collie", dog: "rex", minutes: 30, walker: "Bo" }
adtb> from d in pets.Dog select { dog: d.name, minutes: sum(w.minutes for w in d.<walks.dog) } order by d.name
{ dog: "fido", minutes: 20 }
{ dog: "rex", minutes: 50 }
adtb> \q
```

`insert walks from (...)` inserts every value the query returns; each
gets its own generated `id`. Since `w.dog` is known to be a dog,
`w.dog.breed` needs no match.

The engine keeps the narrower promise too. Turning `rex` into a cat
would leave two walks pointing at something that is not a dog, so it
is refused:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> update d in pets.Dog where d.name == "rex" set Cat { d with lives: 9, indoor: true }
constraint `shelter::walks.Walk.dog` is violated by 2 rows
adtb> \q
```

## Deleting what is referred to

What happens to a reference when its target is deleted is written on
the field. Without anything written, the delete is refused while a
reference remains. `on delete cascade` deletes the referring rows
with it:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> delete p in pets where p.name == "tom"
constraint `shelter::adoptions.Adoption.pet` is violated by 1 row
adtb> delete d in adopters where d.name == "Ann"
constraint `shelter::adoptions.Adoption.adopter` is violated by 2 rows
adtb> delete d in pets.Dog where d.name == "fido"
adtb> from w in walks select { dog: w.dog.name, walker: w.walker.name } order by (w.dog.name, w.walker.name)
{ dog: "rex", walker: "Ann" }
{ dog: "rex", walker: "Bo" }
adtb> \q
```

Deleting `fido` deleted his walk too. Two other actions exist:
`on delete clear`, for an `Option (Key ...)` field, sets it to `None`,
and `on delete retag C { ... }` moves the referring row to another
constructor. They are described in
[the specification](../../spec/writes.md#7-delete-actions).

## Checks and constraints

A `check` is a condition on each row of a relation, or of some of its
constructors: `check sensible_age on pets (...)` names the row's fields
directly. A `constraint` is a query that must return nothing; the rows
it returns are the violations. `one_adopter_per_phone` returns every
adopter who shares a phone number with another one:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> insert pets.Rabbit { name: "old", age: 50, size: Small, diet: Normal }
constraint `shelter::sensible_age` is violated by 1 row
adtb> update a in adopters where a.name == "Bo" set Adopter { a with phone: Some { value: "555-0100" } }
constraint `shelter::one_adopter_per_phone` is violated by 2 rows
adtb> update a in adopters where a.name == "Bo" set Adopter { a with phone: Some { value: "555-0101" } }
adtb> from a in adopters select a order by key a
Adopter { email: "ann@example.com", name: "Ann", phone: Some { value: "555-0100" } }
Adopter { email: "bo@example.com", name: "Bo", phone: Some { value: "555-0101" } }
Adopter { email: "cy@example.com", name: "Cy", phone: None }
adtb> \q
```

The engine checks every constraint at the end of every statement, and
a statement that leaves one violated changes nothing.

Why a constraint and not `unique (phone)` on `adopters`? There is no
NULL, so `None` is a value like any other, and a unique would let only
one adopter have no phone. The constraint skips `None`.

The `index (phone)` clause is there because the engine admits a
constraint only when it can check it without reading the whole
database on every write. Without the index it refuses the schema:

```adt nophone.adt
module shelter

type Adopter = { email: Text, name: Text, phone: Option Text }
relation adopters : Adopter key email

constraint one_adopter_per_phone =
  from a in adopters, b in adopters
  where a.phone != None and a.phone == b.phone and key a != key b
  select a
```

<!-- exit 1 -->
```console
$ adtb --create --schema nophone.adt nophone.db
nophone.db: migration 1 is refused: constraint shelter::one_adopter_per_phone is not admitted: changes to shelter::adopters: index (phone) in shelter::adopters is missing
```

The refusal names the index it needs. [Writing
constraints](../guides/writing-constraints.md) shows more of what the
engine admits, and the rules are in
[the specification](../../spec/constraints.md).

Every text so far ran as a transaction of its own. [Page
6](6-transactions.md) groups several into one.

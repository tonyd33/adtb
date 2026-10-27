# 2. Sums

On [page 1](1-schema.md) a pet had `species: Text`. That lets in any
text, and a fact that only cats have, such as how many lives they have
left, would need a field every pet carries. This page makes `Pet` a
sum: a type whose values are one of several constructors, each with
its own fields.

## The schema

Replace `shelter.adt` with this. Kennels and donations have done their
job and do not come back:

```adt shelter.adt
module shelter

type Adopter = { email: Text, name: Text, phone: Option Text }
relation adopters : Adopter key email

type Size ordered = Small | Medium | Large
type Diet = Normal | Special { food: Text }

type Pet { name: Text, age: I64, size: Size, diet: Diet }
  = Cat { lives: I64, indoor: Bool }
  | Dog { breed: Text }
  | Rabbit
relation pets : Pet key name
```

`Pet` has three constructors, `Cat`, `Dog` and `Rabbit`, separated by
`|`. The record before the `=` is the header: fields every constructor
has. A cat has a name, an age, a size, a diet, a number of lives and
whether it stays indoors. A dog has the four header fields and a breed.
A rabbit has only the header fields.

`pets` is still one relation with one key, `name`. Every row is
exactly one of the three constructors, and no two rows share a name
whatever their constructors, so a cat and a dog cannot both be called
`tom`.

The other two sums are smaller:

- `Size` is an enumeration: constructors with no fields. `ordered`
  makes its values comparable, in the order they are declared, so
  `Small < Large`.
- `Diet` is a sum used as a field type. A pet's diet is either
  `Normal` or `Special` with the food it needs.

`phone: Option Text` is how a field that may be missing is written.
`Option` is a sum the prelude declares, `None | Some { value: a }`, so
an adopter without a phone has `None`, and one with a phone has
`Some { value: "555-0100" }`. There is no NULL.

Since the schema changed, start again: delete `shelter.db` and
`shelter.db-log` and create the database.

## Insert into a constructor

An insert into `pets` names the constructor with `pets.Cat`,
`pets.Dog` or `pets.Rabbit`, and gives the header fields and that
constructor's fields:

```console
$ adtb --create --schema shelter.adt shelter.db
adtb> insert pets.Cat { name: "tom", age: 3, size: Small, diet: Normal, lives: 9, indoor: true }
adtb> insert pets.Dog { name: "rex", age: 5, size: Large, diet: Normal, breed: "collie" }
adtb> insert pets.Rabbit { name: "bun", age: 1, size: Small, diet: Special { food: "hay" } }
adtb> insert adopters { email: "ann@example.com", name: "Ann", phone: Some { value: "555-0100" } }
adtb> insert adopters { email: "bo@example.com", name: "Bo", phone: None }
adtb> \q
```

The engine checks the value against the constructor you name:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> insert pets { name: "kit", age: 1, size: Small, diet: Normal, lives: 9, indoor: false }
a single row is inserted into one constructor of Pet: name it, or insert constructor values with `from (...)`
adtb> insert pets.Dog { name: "kit", age: 1, size: Small, diet: Normal, lives: 9, indoor: false }
`Dog` has no field `lives`
adtb> insert pets.Cat { name: "kit", age: 1, size: Tiny, diet: Normal, lives: 9, indoor: false }
expected Size, found Tiny
adtb> insert adopters { email: "cy@example.com", name: "Cy", phone: "555-0102" }
expected Option Text, found Text
adtb> \q
```

The last insert is refused because `"555-0102"` is a `Text`, not an
`Option Text`: you write `Some { value: "555-0102" }`.

To insert rows of several constructors at once, give a set of
constructor values with `from`:

```console
$ adtb shelter.db
adtb> insert pets from [Cat { name: "kit", age: 1, size: Small, diet: Normal, lives: 9, indoor: false }, Dog { name: "fido", age: 2, size: Medium, diet: Special { food: "low fat" }, breed: "pug" }]
adtb> from p in pets select p order by p.name
Rabbit { name: "bun", age: 1, size: Small, diet: Special { food: "hay" } }
Dog { name: "fido", age: 2, size: Medium, diet: Special { food: "low fat" }, breed: "pug" }
Cat { name: "kit", age: 1, size: Small, diet: Normal, lives: 9, indoor: False }
Dog { name: "rex", age: 5, size: Large, diet: Normal, breed: "collie" }
Cat { name: "tom", age: 3, size: Small, diet: Normal, lives: 9, indoor: True }
adtb> \q
```

A whole row prints as its constructor and its fields, header fields
first, in the order the type declares them.

## Read constructors

Over `pets` you can read the header fields of any row directly. A
field only one constructor has is read over that constructor's rows,
`pets.Cat`, or over a set of constructors, `pets.(Cat | Rabbit)`:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> from p in pets where p.size < Large select p.name
"bun"
"fido"
"kit"
"tom"
adtb> from p in pets select p.lives
Pet has no field `lives`; only header fields are reached on the whole sum: match on the constructor
adtb> from c in pets.Cat select { name: c.name, lives: c.lives } order by c.name
{ lives: 9, name: "kit" }
{ lives: 9, name: "tom" }
adtb> from p in pets.(Cat | Rabbit) select p.name
"bun"
"kit"
"tom"
adtb> from p in pets where p.diet != Normal select p.name
"bun"
"fido"
adtb> from p in pets select { name: p.name, kind: tag(p) } order by p.name
{ kind: Rabbit, name: "bun" }
{ kind: Dog, name: "fido" }
{ kind: Cat, name: "kit" }
{ kind: Dog, name: "rex" }
{ kind: Cat, name: "tom" }
adtb> \q
```

`p.lives` over all pets is refused: a dog has no lives, and the engine
will not guess a value for it. To read each constructor's own fields
in one query you `match` on the row, which [page 4](4-match.md) covers.
`tag(p)` is the constructor a row has.

## Change a row's constructor

`update` gives each row it touches a whole new value. `Cat { c with
age: 4 }` is the cat `c` with its age replaced. The new value may be
of another constructor, which moves the row there; this is called a
retag:

```console
$ adtb shelter.db
adtb> update c in pets.Cat where c.name == "tom" set Cat { c with age: 4 }
adtb> update r in pets.Rabbit where r.name == "bun" set Dog { r with breed: "lop" }
adtb> from d in pets.Dog select d order by d.name
Dog { name: "bun", age: 1, size: Small, diet: Special { food: "hay" }, breed: "lop" }
Dog { name: "fido", age: 2, size: Medium, diet: Special { food: "low fat" }, breed: "pug" }
Dog { name: "rex", age: 5, size: Large, diet: Normal, breed: "collie" }
adtb> update d in pets.Dog where d.name == "bun" set Rabbit { d with age: 2 }
adtb> from p in pets select { name: p.name, kind: tag(p) } order by p.name
{ kind: Rabbit, name: "bun" }
{ kind: Dog, name: "fido" }
{ kind: Cat, name: "kit" }
{ kind: Dog, name: "rex" }
{ kind: Cat, name: "tom" }
adtb> \q
```

`with` keeps the header fields and any fields the two constructors
share, drops the fields the new constructor lacks, and takes the
fields it adds from what you write. The row keeps its key, so it is
still the pet named `bun`. An update never changes a key.

## Delete

`delete` removes the rows its condition picks:

```console
$ adtb shelter.db
adtb> delete p in pets.Dog where p.age > 4
adtb> from p in pets select p.name
"bun"
"fido"
"kit"
"tom"
adtb> \q
```

The rules for inserts, updates and retags are in
[the specification](../../spec/writes.md). Next, [page 3](3-queries.md)
asks more of the pets.

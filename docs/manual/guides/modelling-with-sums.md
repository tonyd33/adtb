# Modelling with sums

This guide is for the moment you sit down with a domain and have to
decide what its types are. adtb gives you three kinds of sum, no NULL
and no collection fields, so a few habits from SQL or document stores
change. The running example is a small veterinary clinic: owners,
their pets, and the pets' vaccinations.

## Choose the level of each sum

A sum can sit at three places in a schema, and each answers a
different question.

- An enumeration is a sum whose constructors carry nothing, used as a
  field type: a pet's species is `Cat`, `Dog` or `Rabbit`.
- A value-level sum is a sum with payloads used as a field type: an
  owner's contact is a phone number, an email address, or nothing.
  Each row holds one value of it, and the value's shape varies.
- A relation-level sum is the type of a relation, with several
  constructors: a pet is living or deceased, and a deceased pet has a
  year of death that a living one does not. Each constructor is its
  own constructor relation, with its own rows, uniques and indexes.

Reach for a relation-level sum when the constructors are kinds of
thing you query, constrain or reference separately, or when a row
moves from one kind to another over its life. Reach for a value-level
sum when the varying part is a detail of one row that nothing else
points at.

Save this as `clinic.adt`:

```adt clinic.adt
module clinic

type Species = Cat | Dog | Rabbit

type Contact = Phone { number: Text } | Email { address: Text } | NoContact

type Owner = { name: Text, contact: Contact, nickname: Option Text }
relation owners : Owner key name

type Pet { id: Key pets, owner: Key owners, name: Text, species: Species }
  = Living
  | Deceased { year: I64 }
relation pets : Pet key id generate uuidv7
  index (owner)

type Vaccination = { pet: Key pets, vaccine: Text, year: I64 }
relation vaccinations : Vaccination key (pet, vaccine)
```

`Pet` has a header, the record before the `=`: every pet has an `id`,
an `owner`, a `name` and a `species`, whichever constructor it is.
`Living` has no fields of its own; `Deceased` adds `year`.

Create the database and add an owner of each contact shape:

```console
$ adtb --create --schema clinic.adt clinic.db
adtb> insert owners { name: "ann", contact: Phone { number: "555-0101" }, nickname: None }
adtb> insert owners { name: "bo", contact: NoContact, nickname: Some { value: "B" } }
adtb> \q
```

## Optional facts

A fact a row may lack has three honest shapes, and NULL is none of
them.

- An `Option` field, when the fact is simply present or absent and
  absence means nothing more: `nickname: Option Text`. A value is
  `None` or `Some { value: ... }`.
- A constructor, when absence and presence are different states with
  different fields: a living pet has no year of death, so `year`
  belongs to `Deceased` and never exists on `Living`. There is no
  `death_year: Option I64` whose `None` you would have to interpret.
- A constructor of a value-level sum, when the absence has a name of
  its own: `NoContact` says the owner gave no contact, which is a
  different fact from "not yet asked".

Reading an optional fact means saying what happens in each case, with
`match`:

```console
$ adtb clinic.db
adtb> from o in owners select match o.nickname | Some { value: n } -> n | None -> o.name
"B"
"ann"
adtb> from o in owners select match o.contact | Phone p -> p.number | Email e -> e.address | NoContact -> "none"
"555-0101"
"none"
adtb> \q
```

[No NULL](../concepts/no-null.md) explains why the engine makes you
write the `None` arm.

## Many-valued facts

A field holds one value: there are no list or set fields. A pet with
several vaccinations is several rows of another relation, each
referring back to the pet. `vaccinations` is keyed by `(pet,
vaccine)`, so a pet has each vaccine once.

A pet's key is generated, so the insert that creates the pet and the
inserts that refer to it run in one `transact` block, where `let`
names the new row and `key p` is its key:

```console
$ adtb clinic.db
adtb> transact
  ...   let p = insert pets.Living { owner: owners.key("ann"), name: "tom", species: Cat }
  ...   insert vaccinations { pet: key p, vaccine: "rabies", year: 2025 }
  ...   insert vaccinations { pet: key p, vaccine: "flu", year: 2026 }
  ...
adtb> insert pets.Living { owner: owners.key("bo"), name: "rex", species: Dog }
adtb> from p in pets select { pet: p.name, shots: from v in p.<vaccinations.pet select v.vaccine } order by p.name
{ pet: "rex", shots: [] }
{ pet: "tom", shots: ["flu", "rabies"] }
adtb> \q
```

`p.<vaccinations.pet` is a backlink: the vaccinations whose `pet`
field refers to `p`. A query may return a nested set, as `shots` is
here, so the many-valued fact comes back attached to its row even
though it is stored apart. A pet with no vaccinations gets an empty
set, not a missing value.

## Choose keys

Every relation has exactly one key, and a reference to a row is its
key. Three kinds of key cover most cases.

- A natural key, when the domain already has a unique name for the
  thing that never changes: `owners` is keyed by `name` here. A
  natural key makes references readable, `owners.key("ann")`, but the
  key of a row never changes, so pick a field that will not need to.
- A generated key, `key id generate uuidv7` or `generate sequence`,
  when nothing in the domain is unique and stable. The engine fills
  the key on insert. Generated values are unique, never reused, and
  not consecutive: do not read meaning into them.
- A composite key, `key (pet, vaccine)`, when a row is identified by
  the things it relates. It is the natural shape for a many-valued
  fact. Build one with named fields:
  `vaccinations.key(pet: k, vaccine: "flu")`.

A key value has the type `Key r` of its relation, and only `r.key(...)`
or `key x` makes one, so a number cannot stand in for a pet:

```console
$ adtb clinic.db -c 'insert vaccinations { pet: pets.key(1), vaccine: "x", year: 1 }'
expected Uuid, found I64
```

`index (owner)` on `pets` makes the lookups "the pets of this owner"
and the backlink `o.<pets.owner` cheap. Index a reference field you
navigate backwards, and any field you filter on often. The full rules
for keys are in the specification's
[keys](../../spec/model.md#7-keys) and
[references](../../spec/model.md#8-references) sections.

## Move a row between constructors

When a pet dies it does not become a different pet: its key, owner,
name and species stay, and it gains a year. Moving a row to another
constructor is a retag, written as an update whose new value is a
value of the other constructor. `{ p with year: 2026 }` copies the
fields of `p` and adds `year`:

```console
$ adtb clinic.db
adtb> update p in pets.Living where p.name == "rex" set Deceased { p with year: 2026 }
adtb> from p in pets select { name: p.name, alive: match p | Living _ -> true | Deceased _ -> false } order by p.name
{ alive: False, name: "rex" }
{ alive: True, name: "tom" }
adtb> from p in pets.Deceased select { name: p.name, year: p.year }
{ name: "rex", year: 2026 }
adtb> \q
```

The row keeps its key, so the vaccinations that refer to rex still
do. A retag is checked like any other write: every constraint on the
new constructor applies to the row as it arrives. See
[retag](../../spec/writes.md#4-retag) for the full rule.

## Enumerations are compared, sums are matched

An enumeration value can be compared with `==`, which reads well in a
filter:

```console
$ adtb clinic.db -c 'from p in pets where p.species == Cat select p.name'
"tom"
```

A sum with payloads is read with `match`, which must name every
constructor. That is what makes adding a constructor later safe: every
query that reads the sum stops checking until it says what to do with
the new case. [Evolving a schema](evolving-a-schema.md) shows that
from the migration side.

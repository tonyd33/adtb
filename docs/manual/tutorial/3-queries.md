# 3. Queries

This page asks questions of the shelter: which pets are young, who
adopted what, how many pets of each size there are. Every query is a
comprehension, and every result is a set.

## The schema

This is the schema of [page 2](2-sums.md) with adoptions and two
functions added. Save it as `shelter.adt`:

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

-- For now an adoption holds the pet's name and the adopter's email.
type Adoption = { pet: Text, adopter: Text, fee: Decimal 2 }
relation adoptions : Adoption key pet

fn is_young (p: Pet) -> Bool = p.age < 2
fn house_cats () -> Set Pet.Cat = from c in pets.Cat where c.indoor select c
```

`fn` declares a function in the schema. Its parameter and result
types are written out. `is_young` takes a pet and returns a `Bool`;
`house_cats` takes nothing and returns a set of cats, so it works as a
named query, a view. `Pet.Cat` is the type of a `Pet` that is a `Cat`.

Create the database with some pets, adopters and adoptions:

```console
$ adtb --create --schema shelter.adt shelter.db
adtb> insert pets.Cat { name: "tom", age: 3, size: Small, diet: Normal, lives: 9, indoor: true }
adtb> insert pets.Cat { name: "kit", age: 1, size: Small, diet: Special { food: "kitten" }, lives: 9, indoor: false }
adtb> insert pets.Dog { name: "rex", age: 5, size: Large, diet: Normal, breed: "collie" }
adtb> insert pets.Dog { name: "fido", age: 2, size: Medium, diet: Special { food: "low fat" }, breed: "pug" }
adtb> insert pets.Dog { name: "max", age: 8, size: Large, diet: Normal, breed: "collie" }
adtb> insert pets.Rabbit { name: "bun", age: 1, size: Small, diet: Normal }
adtb> insert adopters { email: "ann@example.com", name: "Ann", phone: Some { value: "555-0100" } }
adtb> insert adopters { email: "bo@example.com", name: "Bo", phone: None }
adtb> insert adopters { email: "cy@example.com", name: "Cy", phone: None }
adtb> insert adoptions { pet: "tom", adopter: "ann@example.com", fee: 40.00 }
adtb> insert adoptions { pet: "rex", adopter: "ann@example.com", fee: 60.00 }
adtb> insert adoptions { pet: "bun", adopter: "bo@example.com", fee: 15.00 }
adtb> \q
```

## Filter and project

`from x in r` binds `x` to each row of `r` in turn, `where` keeps the
bindings its condition holds for, and `select` gives the value each
kept binding contributes to the result. Conditions combine with `and`,
`or` and `not`:

```console
$ adtb shelter.db
adtb> from p in pets where p.age >= 2 and p.size != Large select p.name
"fido"
"tom"
adtb> from p in pets select p.size
Small
Medium
Large
adtb> from p in pets select { name: p.name, years_left: 15 - p.age } order by p.name
{ name: "bun", years_left: 14 }
{ name: "fido", years_left: 13 }
{ name: "kit", years_left: 14 }
{ name: "max", years_left: 7 }
{ name: "rex", years_left: 10 }
{ name: "tom", years_left: 12 }
adtb> from p in pets where is_young(p) select p.name
"bun"
"kit"
adtb> from c in house_cats() select c.name
"tom"
adtb> \q
```

Six pets have only three sizes, so `select p.size` returns three
values: a result is a set, and equal values are one element. There
are no duplicate rows to remove, and no `distinct` to write.

## Join

Several generators in one `from` range over every combination of their
rows, and `where` picks the combinations that belong together. Each
generator can use the ones before it:

```console
$ adtb shelter.db
adtb> from a in adoptions, d in adopters where a.adopter == d.email select { pet: a.pet, adopter: d.name } order by a.pet
{ adopter: "Bo", pet: "bun" }
{ adopter: "Ann", pet: "rex" }
{ adopter: "Ann", pet: "tom" }
adtb> from a in adoptions, d in adopters, p in pets where a.adopter == d.email and a.pet == p.name select { adopter: d.name, pet: p.name, age: p.age } order by p.name
{ adopter: "Bo", age: 1, pet: "bun" }
{ adopter: "Ann", age: 5, pet: "rex" }
{ adopter: "Ann", age: 3, pet: "tom" }
adtb> \q
```

Matching on emails and names by hand works, but nothing yet stops an
adoption from naming a pet that does not exist. [Page
5](5-references-and-constraints.md) turns these fields into
references, which the engine keeps valid and lets you follow with a
dot.

## Nested results and emptiness

A `select` can hold another query, so a result can be nested. An
adopter with no adoptions gets an empty set, `[]`, not a missing row:

```console
$ adtb shelter.db
adtb> from d in adopters select { name: d.name, pets: from a in adoptions where a.adopter == d.email select a.pet } order by d.name
{ name: "Ann", pets: ["rex", "tom"] }
{ name: "Bo", pets: ["bun"] }
{ name: "Cy", pets: [] }
adtb> from p in pets where not exists a in adoptions where a.pet == p.name select p.name
"fido"
"kit"
"max"
adtb> \q
```

`exists x in r where c` is true when some row of `r` satisfies `c`.
`not exists` finds the pets still waiting for a home.

## Aggregates

An aggregate is a call whose argument is a generator: `count(x for x
in r where c)`. It counts or adds up every binding, so two adoptions
with the same fee both count:

```console
$ adtb shelter.db
adtb> count(p for p in pets.Dog)
3
adtb> sum(a.fee for a in adoptions)
115.00
adtb> max(p.age for p in pets)
Some { value: 8 }
adtb> from d in adopters select { name: d.name, adopted: count(a for a in adoptions where a.adopter == d.email), spent: sum(a.fee for a in adoptions where a.adopter == d.email) } order by d.name
{ adopted: 2, name: "Ann", spent: 100.00 }
{ adopted: 1, name: "Bo", spent: 15.00 }
{ adopted: 0, name: "Cy", spent: 0.00 }
adtb> \q
```

A query can be a whole text on its own, as `count(...)` is here. Over
an empty range `count` and `sum` are `0`, but `min` and `max` have no
answer, so they return an `Option`: `Some { value: 8 }` here, `None`
when there are no rows.

## Group

`group x by k into g` collects the bindings into one group per value
of `k`. After it, `g.key` is that value and `g` is the group's rows:

```console
$ adtb shelter.db
adtb> from p in pets group p by p.size into g select { size: g.key, n: count(x for x in g), oldest: max(x.age for x in g) } order by g.key
{ n: 3, oldest: 3, size: Small }
{ n: 1, oldest: 2, size: Medium }
{ n: 2, oldest: 8, size: Large }
adtb> from d in pets.Dog group d by d.breed into g select { breed: g.key, names: from x in g select x.name } order by g.key
{ breed: "collie", names: ["max", "rex"] }
{ breed: "pug", names: ["fido"] }
adtb> \q
```

A group is never empty, so `max(x.age for x in g)` is an `I64`, not
an `Option`.

## Order and pages

`order by` sorts the final result. A list of keys in parentheses sorts
by the first, then the next on a tie; `desc` reverses one. An ordering
key reads the selected value, so to sort by age you select something
that holds the age. You still write the key through the row variable,
`p.age`; it is accepted because the selected record holds `p.age`
unchanged in its field `age`:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> from p in pets select { name: p.name, age: p.age } order by (p.age desc, p.name)
{ age: 8, name: "max" }
{ age: 5, name: "rex" }
{ age: 3, name: "tom" }
{ age: 2, name: "fido" }
{ age: 1, name: "bun" }
{ age: 1, name: "kit" }
adtb> from p in pets select p.name order by p.age
an ordering key reads only the selected value, or fields the selected value holds unchanged
adtb> from p in pets select p order by p.name take 2
Rabbit { name: "bun", age: 1, size: Small, diet: Normal }
Dog { name: "fido", age: 2, size: Medium, diet: Special { food: "low fat" }, breed: "pug" }
adtb> from p in pets select p order by p.name take 2 after Some { value: "fido" }
Cat { name: "kit", age: 1, size: Small, diet: Special { food: "kitten" }, lives: 9, indoor: False }
Dog { name: "max", age: 8, size: Large, diet: Normal, breed: "collie" }
adtb> \q
```

`take n` keeps the first `n` results. `after` starts after a given
ordering value: the value of the `order by` key, here `p.name`, not
a row. The first page ended at `fido`, so the next page passes
`fido`'s name. It takes an `Option`, and `None` means the first page. There is no
offset. Both need an order with no ties, so the query selects whole
rows and orders by a key.

`top n by k (q)` is the first `n` results of `q`, as a set you can use
anywhere, inside another query included:

```console
$ adtb shelter.db
adtb> top 2 by (p.age desc, key p) (from p in pets.Dog select p)
Dog { name: "max", age: 8, size: Large, diet: Normal, breed: "collie" }
Dog { name: "rex", age: 5, size: Large, diet: Normal, breed: "collie" }
adtb> \q
```

## Other forms

```console
$ adtb shelter.db
adtb> (from c in pets.Cat select c.name) union (from r in pets.Rabbit select r.name)
"bun"
"kit"
"tom"
adtb> from p in pets select { name: p.name, stage: if p.age < 2 then "young" else "grown" } order by p.name
{ name: "bun", stage: "young" }
{ name: "fido", stage: "grown" }
{ name: "kit", stage: "young" }
{ name: "max", stage: "grown" }
{ name: "rex", stage: "grown" }
{ name: "tom", stage: "grown" }
adtb> "bun" ++ " the " ++ "rabbit"
"bun the rabbit"
adtb> \q
```

`union` joins two sets. `if c then a else b` chooses a value. `++`
joins texts. The meaning of every query form is in
[the specification](../../spec/queries.md), and the functions you can
call are in [the prelude reference](../reference/prelude.md).

Every query so far read only header fields, or the fields of one
constructor. [Page 4](4-match.md) reads each constructor's own fields
in one query.

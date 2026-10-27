# 4. Match

A cat has lives, a dog has a breed, and a rabbit has neither. To read
the fields of whichever constructor a row has, you `match` on it. A
match lists one arm per constructor, and the engine checks that the
arms cover every constructor, so adding a constructor later shows you
every query that has to handle it.

## The schema

This is the schema of [page 2](2-sums.md) with a named set of
constructors and two functions that match. Save it as `shelter.adt`:

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

set Furry of pets = Cat | Dog

fn species (p: Pet) -> Text = match p
  | Cat _ -> "cat"
  | Dog _ -> "dog"
  | Rabbit _ -> "rabbit"

fn food (d: Diet) -> Text = match d
  | Normal -> "standard"
  | Special s -> s.food
```

Each arm is `| pattern -> expression`. `Cat _` matches any cat and
binds nothing. `Special s` matches a special diet and binds it to `s`,
so `s.food` is there. `Normal` alone matches the constructor with no
fields. `Rabbit` declares no fields of its own but has the header
fields, so a pattern for it is `Rabbit _` or `Rabbit r`.

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
adtb> \q
```

## Match in a query

```console
$ adtb shelter.db
adtb> from p in pets select { name: p.name, species: species(p), food: food(p.diet) } order by p.name
{ food: "standard", name: "bun", species: "rabbit" }
{ food: "low fat", name: "fido", species: "dog" }
{ food: "kitten", name: "kit", species: "cat" }
{ food: "standard", name: "max", species: "dog" }
{ food: "standard", name: "rex", species: "dog" }
{ food: "standard", name: "tom", species: "cat" }
adtb> from p in pets select { name: p.name, about: match p | Cat c -> if c.indoor then "indoor cat" else "outdoor cat" | Dog d -> d.breed | Rabbit _ -> "rabbit" } order by p.name
{ about: "rabbit", name: "bun" }
{ about: "pug", name: "fido" }
{ about: "outdoor cat", name: "kit" }
{ about: "collie", name: "max" }
{ about: "collie", name: "rex" }
{ about: "indoor cat", name: "tom" }
adtb> from a in adopters select { name: a.name, phone: match a.phone | Some s -> s.value | None -> "no phone" } order by a.name
{ name: "Ann", phone: "555-0100" }
{ name: "Bo", phone: "no phone" }
adtb> \q
```

In the arm `Cat c`, `c` is the row as a cat, so `c.indoor` is there;
in `Dog d`, `d.breed` is. A match on `Option` is how you read a field
that may be missing: there is no NULL to test for, only `None` and
`Some`.

A match over the rows of a relation is a union of one query per
constructor: the `Cat` arm reads only the cats, and so on. Each arm's
value must have one common type.

## What a match must cover

```console
$ adtb shelter.db
adtb> from p in pets select match p | Cat c -> c.name | Dog d -> d.name
match does not cover `Rabbit _`
adtb> from p in pets select match p | Cat c -> c.lives | Dog d -> d.breed | Rabbit _ -> 0
the arms have no common type: I64 and Text
adtb> from p in pets select match p | Cat c -> c.name | Dog d -> d.name | Rabbit r -> r.name | Dog d -> "again"
this arm is unreachable: the arms before it cover every value it matches
adtb> \q
```

A missing constructor is an error, not a silent gap, and so is an arm
that can never be chosen.

## Patterns

A pattern can look inside the value it matches:

- `Cat { name, indoor: true }` is a record pattern. `name` alone binds
  the field under its own name; `indoor: true` matches only when the
  field is `true`. Fields you leave out match anything.
- Patterns nest: `Dog { diet: Special { food } }` matches a dog on a
  special diet and binds `food`.
- `A | B ->` is an or-pattern: one arm for several patterns, which must
  bind the same names at the same types.
- `if` after a pattern is a guard: the arm is chosen only when the
  condition holds, so a guarded arm does not count toward covering
  its constructor.
- Literals match numbers and texts, and over a number or a text the
  last arm is `_` or a name, which matches anything left.

```console
$ adtb shelter.db
adtb> from p in pets select match p | Cat { name, indoor: true } -> name ++ " stays in" | Cat { name } -> name ++ " goes out" | Dog { name, breed } -> name ++ " the " ++ breed | Rabbit r -> r.name ++ " the rabbit"
"bun the rabbit"
"fido the pug"
"kit goes out"
"max the collie"
"rex the collie"
"tom stays in"
adtb> from p in pets select match p | Cat { name, diet: Special { food } } | Dog { name, diet: Special { food } } -> name ++ " eats " ++ food | Cat _ | Dog _ | Rabbit _ -> "-"
"-"
"fido eats low fat"
"kit eats kitten"
adtb> from p in pets select match p | Dog d if d.age > 4 -> d.name ++ " is a senior dog" | Dog d -> d.name ++ " is a dog" | Cat c -> c.name | Rabbit r -> r.name
"bun"
"fido is a dog"
"kit"
"max is a senior dog"
"rex is a senior dog"
"tom"
adtb> from p in pets select match p.age | 1 -> "one" | 2 -> "two" | n -> "older"
"older"
"one"
"two"
adtb> \q
```

Arms are tried in order, so the record pattern with `indoor: true`
comes before the one that takes any cat.

## Named sets

`set Furry of pets = Cat | Dog` names a set of constructors of `pets`.
`pets.Furry` reads those rows, and `Furry f` is a pattern that covers
both:

```console
$ adtb shelter.db
adtb> from p in pets select match p | Furry f -> f.name ++ " is furry" | Rabbit r -> r.name ++ " is fluffy"
"bun is fluffy"
"fido is furry"
"kit is furry"
"max is furry"
"rex is furry"
"tom is furry"
adtb> from f in pets.Furry select { name: f.name, kind: tag(f) } order by f.name
{ kind: Dog, name: "fido" }
{ kind: Cat, name: "kit" }
{ kind: Dog, name: "max" }
{ kind: Dog, name: "rex" }
{ kind: Cat, name: "tom" }
adtb> \q
```

`pets.(Cat | Dog)` reads the same rows without a name. A named set is
worth declaring when several queries mean the same group.

## Wildcards

In a text you type at the shell, `_` may stand for the constructors
you did not name:

```console
$ adtb shelter.db
adtb> from p in pets select match p | Cat c -> "cat" | _ -> "not a cat"
"cat"
"not a cat"
adtb> \q
```

In the schema it may not. A function, a constraint or anything else
that lives in the database names every constructor, so that adding
one is caught where it matters. Save this as `wild.adt`:

```adt wild.adt
module shelter

type Pet { name: Text } = Cat { lives: I64 } | Dog { breed: Text } | Rabbit
relation pets : Pet key name

fn species (p: Pet) -> Text = match p
  | Cat _ -> "cat"
  | _ -> "other"
```

```console
$ adtb --create --schema wild.adt wild.db
wild.db: migration 1 is refused: `_` and a binder do not match a value of a sum; name the constructors of Pet
```

When the schema is refused the shell creates no database. Over a
number, a text or another scalar, `_` is always allowed, since no one
can list every number.

The rules of matching and coverage are in
[the specification](../../spec/queries.md#5-match). Next,
[page 5](5-references-and-constraints.md) links pets to their
adopters with references and adds constraints.

# Relations are sums

In a SQL database a table has one row shape. When the things it holds
come in several kinds, you choose between nullable columns that only
some rows fill, a `type` column with checks that tie it to the columns
it allows, or one table per kind joined to a parent table on a shared
key. Each choice leaves part of the model to convention: nothing stops
a row from having the columns of two kinds, or a subtype row from
losing its parent.

adtb puts the kinds into the type. The element type of a relation is a
sum: a list of constructors, each with its own fields. Every row is a
value of exactly one constructor.

## One relation, several constructors

Save this as `zoo.adt`:

```adt zoo.adt
module zoo

type Water = Fresh | Salt

type Animal { name: Text, born: I64 }
  = Bird { wingspan: I64 }
  | Fish { water: Water }
  | Snake

relation animals : Animal key name

type Keeper = { name: Text, since: I64 }
relation keepers : Keeper key name
```

`Animal` has three constructors. The record before the `=` is the
header: fields every constructor has, here `name` and `born`. Each
constructor adds its own fields, and `Snake` adds none. `Water` is a
sum too, with two constructors and no fields; a sum like that is an
enumeration.

```console
$ adtb --create --schema zoo.adt zoo.db
adtb> insert animals.Bird { name: "polly", born: 2019, wingspan: 30 }
adtb> insert animals.Fish { name: "nemo", born: 2021, water: Salt }
adtb> insert animals.Snake { name: "kaa", born: 2015 }
adtb> from a in animals select a order by a.name
Snake { name: "kaa", born: 2015 }
Fish { name: "nemo", born: 2021, water: Salt }
Bird { name: "polly", born: 2019, wingspan: 30 }
```

Each row prints with its constructor. There is no column a snake
leaves empty: a snake has no wingspan field at all.

## Constructor relations

The rows of one constructor form a constructor relation, written
`animals.Bird`. A list of constructors in parentheses is a constructor
set, the union of their constructor relations. Over the whole relation
you can read the header fields; a constructor's own fields need a
range that names the constructor, or a `match`
([Tutorial 4](../tutorial/4-match.md)).

```console
$ adtb zoo.db
adtb> from b in animals.Bird select b.wingspan
30
adtb> from a in animals.(Bird | Snake) select a.name order by a.name
"kaa"
"polly"
adtb> from a in animals where a.born < 2020 select a.name order by a.name
"kaa"
"polly"
adtb> from a in animals select a.wingspan
Animal has no field `wingspan`; only header fields are reached on the whole sum: match on the constructor
```

A constructor relation behaves like a table of its own: you can range
over it, count it and index it. Unlike a table per subtype, it is not
a separate thing to keep in step with a parent. The relation is the
disjoint union of its constructor relations, and the engine keeps it
that way.

## Disjoint, total, one key

Every row belongs to exactly one constructor, and the key ranges over
all of them. A name used by a bird cannot be used by a snake:

```console
$ adtb zoo.db -c 'insert animals.Snake { name: "polly", born: 2000 }'
`zoo::animals` already holds a row with key `animals.key("polly")`
```

Changing a row's kind is an update whose new value has another
constructor. It is called a retag. The row keeps its key, so anything
that refers to it still does:

```console
$ adtb zoo.db
adtb> update a in animals.Snake set Bird { a with wingspan: 0 }
adtb> from a in animals.Bird select a order by a.name
Bird { name: "kaa", born: 2015, wingspan: 0 }
Bird { name: "polly", born: 2019, wingspan: 30 }
adtb> { birds: count(a for a in animals.Bird), fish: count(a for a in animals.Fish), snakes: count(a for a in animals.Snake) }
{ birds: 2, fish: 1, snakes: 0 }
```

`Bird { a with wingspan: 0 }` is the snake's value with its fields
carried over and the bird's field added.

## A record is a sum of one constructor

`keepers` holds records. A record type is a sum with one constructor
named after the type, so `Keeper` has the constructor `Keeper`, and
`keepers.Keeper` is the whole relation:

```console
$ adtb zoo.db
adtb> insert keepers { name: "ann", since: 2010 }
adtb> from k in keepers.Keeper select k
Keeper { name: "ann", since: 2010 }
adtb> from k in keepers select match k | Keeper x -> x.since
2010
```

Everything this page says about sums holds for records too. You only
write the constructor name when a relation has more than one.

## Sums as field types

`Water` is used as the type of a field. A sum can sit at either level:
as the element type of a relation, where each constructor gets a
constructor relation, or as the type of a field, where it is part of a
row's value. The same `match` reads both.

```console
$ adtb zoo.db -c 'from f in animals.Fish where f.water == Salt select f.name'
"nemo"
```

## Results are sets

A relation never holds two equal rows, and neither does a query
result. Projecting a field that several rows share gives each value
once:

```console
$ adtb zoo.db -c 'from b in animals.Bird select tag(b)'
Bird
```

`tag(b)` is the constructor of `b` as a value. Both birds have the
same one, so the result has one row.

## Coming from SQL

| SQL design | In adtb |
| --- | --- |
| Nullable columns filled only for some kinds | One constructor per kind, each with the fields it has |
| A `type` column with checks on the other columns | The constructor itself; `tag(x)` reads it |
| A table per subtype sharing a parent's key | Constructor relations of one relation, under one key |
| A `CHECK` that a row is exactly one kind | Always true: every row has exactly one constructor |

The full rules are in [the model](../../spec/model.md#2-sums) and
[constructor relations](../../spec/model.md#63-constructor-relations-and-constructor-sets).

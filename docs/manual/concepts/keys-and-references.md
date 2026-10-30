# Keys and references

A row in adtb is identified by its key and by nothing else. There is
no hidden row id or object id: two rows of a relation never agree on
the key, and a reference to a row is that row's key, held in a field
and checked by the engine.

## Declared keys

Every relation declares exactly one key with `key f`, or
`key (f, g)` for a key of several fields. The key names fields the
type declares. In a relation of a sum, the key is in the header, so
one key covers every constructor. Save this as `library.adt`:

```adt library.adt
module library

type Member = { card: Text, name: Text }
relation members : Member key card

type Book { isbn: Text, title: Text }
  = Shelved { shelf: Text }
  | Lent { to: Key members }
relation books : Book key isbn

type Hold = { book: Key books.Shelved, by: Key members on delete cascade }
relation holds : Hold key (book, by)
```

`members` is keyed by a library card number and `books` by ISBN:
values that mean something outside the database. When no such value
exists, a relation can generate its key instead, with
`key id generate sequence` or `key id generate uuidv7` on a field
`id: Key r`. A generated key is promised to be distinct from every
other, not to be consecutive or to follow insertion order.

<!-- exit 1 -->
```console
$ adtb --create --schema library.adt library.db
adtb> insert members { card: "c1", name: "Ann" }
adtb> insert members { card: "c2", name: "Bo" }
adtb> insert books.Shelved { isbn: "111", title: "Dune", shelf: "A1" }
adtb> insert books.Lent { isbn: "222", title: "Emma", to: members.key("c2") }
adtb> insert members { card: "c1", name: "Cy" }
`library::members` already holds a row with key `members.key("c1")`
```

## Key types

The key of a row of `books` has type `Key books`. It is a type of its
own, not the `Text` the ISBN is made of. You build one from its value
with `books.key("111")`, take the key of a bound row with `key b`, and
read the value back with `value(k)`. A composite key is built with
named fields:

<!-- exit 1 -->
```console
$ adtb library.db
adtb> books.key("111")
books.key("111")
adtb> from b in books where key b == "111" select b.title
Key books and Text cannot be compared
adtb> from b in books where key b == books.key("111") select b.title
"Dune"
adtb> from b in books select value(key b) order by value(key b)
"111"
"222"
adtb> holds.key(book: books.key("111"), by: members.key("c1"))
expected Key books.Shelved, found Key books
```

The last line is refused because of what a reference may point at,
the subject of the next section.

## References are typed by constructor set

A field of type `Key members` is a reference: it holds the key of a
row of `members`, and the engine refuses any write that would leave it
pointing at no row. A reference can also be narrowed to some
constructors. `Hold.book` is a `Key books.Shelved`, so a hold can only
name a book that is on the shelf. A key built with `books.key(...)`
is a `Key books`, which could be any book, so it does not fit; the key
of a row ranged over `books.Shelved` does:

<!-- exit 1 -->
```console
$ adtb library.db
adtb> insert holds from (from b in books.Shelved where b.title == "Dune" select { book: key b, by: members.key("c1") })
adtb> insert holds from (from b in books.Lent select { book: key b, by: members.key("c1") })
expected Key books.Shelved, found Key books.Lent
adtb> update b in books.Shelved set Lent { b with to: members.key("c1") }
constraint `library::holds.Hold.book` is violated by 1 row
```

Lending the held book is refused: the retag would move the row out of
`Shelved` while a hold still points at it as a shelved book. The reference is a
foreign key to one constructor of `books`, not to the whole relation.

## Following references

A path follows a reference to the row it names: `h.book.title` reads
the title of the held book. A backlink goes the other way:
`m.<books.Lent.to` is the set of rows of `books.Lent` whose `to` field
names `m`.

```console
$ adtb library.db
adtb> from h in holds select { title: h.book.title, by: h.by.name }
{ by: "Ann", title: "Dune" }
adtb> from m in members select { name: m.name, lent: from b in m.<books.Lent.to select b.title } order by m.name
{ lent: [], name: "Ann" }
{ lent: ["Emma"], name: "Bo" }
```

## Deleting a referenced row

A reference field states what happens when its target is deleted. With
no action, the delete is refused while a reference names the row.
`on delete cascade` deletes the referring rows with it, and
`on delete clear`, on an `Option (Key ...)` field, sets the field to
`None`:

<!-- exit 1 -->
```console
$ adtb library.db
adtb> delete m in members where m.name == "Bo"
constraint `library::books.Lent.to` is violated by 1 row
adtb> delete m in members where m.name == "Ann"
adtb> count(h for h in holds)
0
```

Bo still has a book out, so he stays. Ann's hold goes with her.

## Where references may be

A reference is a direct field of a relation's row, of type `Key r` or
`Option (Key r)`: a header field, or a field of one constructor, as
`Lent { to: Key members }` is. It cannot hide inside a value-level sum or a record
field, and no field holds a collection. A fact with many values, such
as the tags of a book or the holds of a member, is a relation whose
rows reference their owner, as `holds` does. The engine refuses the
other forms when the schema is created:

```adt library-bad.adt
module library

type Member = { card: Text, name: Text, tags: Set Text }
relation members : Member key card
```

<!-- exit 1 -->
```console
$ adtb --create --schema library-bad.adt bad.db
bad.db: migration 1 is refused:
  a field never holds a set; a many-valued fact is a relation
  no type `Member`
```

The second line follows from the first: the refused type is not
declared, so the relation that names it fails too. Fix the first
error and the second goes.

The rules are in [keys](../../spec/model.md#7-keys) and
[references](../../spec/model.md#8-references).

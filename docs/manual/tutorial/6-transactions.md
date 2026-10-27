# 6. Transactions

Every text you have typed so far ran as a transaction of its own. This
page groups writes: several texts that succeed or fail together,
`transact` blocks that make several changes as one, and interactive
transactions that you commit or roll back by hand.

adtb has one isolation level, serializable: whatever runs at the same
time, the result is that of some order in which the transactions ran
one after another. There is nothing to configure. [Serializable
transactions](../concepts/serializable-transactions.md) explains what
that means for an application.

## The schema

This page uses the schema of [page 5](5-references-and-constraints.md).
Save it as `shelter.adt`:

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
```

```console
$ adtb --create --schema shelter.adt shelter.db
adtb> insert pets.Cat { name: "tom", age: 3, size: Small, diet: Normal, lives: 9, indoor: true }
adtb> insert pets.Cat { name: "kit", age: 1, size: Small, diet: Special { food: "kitten" }, lives: 9, indoor: false }
adtb> insert pets.Dog { name: "rex", age: 5, size: Large, diet: Normal, breed: "collie" }
adtb> insert adopters { email: "ann@example.com", name: "Ann", phone: Some { value: "555-0100" } }
adtb> insert adopters { email: "bo@example.com", name: "Bo", phone: Some { value: "555-0101" } }
adtb> insert adopters { email: "cy@example.com", name: "Cy", phone: None }
adtb> insert adoptions { pet: pets.key("tom"), adopter: adopters.key("ann@example.com"), fee: 40.00 }
adtb> \q
```

## Statements

A statement is one change to the database, applied all at once. The
engine checks every key, reference, check and constraint at the end of
each statement, and a statement that would leave one violated changes
nothing.

Ann and Bo want to swap phone numbers. As two statements it cannot be
done: whichever runs first leaves two adopters with one number, and
`one_adopter_per_phone` refuses it.

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> update a in adopters where a.name == "Ann" set Adopter { a with phone: Some { value: "555-0101" } }
constraint `shelter::one_adopter_per_phone` is violated by 2 rows
adtb> \q
```

## Transact blocks

A `transact` block is one statement made of several writes. Its items
are indented under it; at the prompt, a blank line ends the block and
runs it. Here both updates apply together, and the constraint is
checked once, after both:

```console
$ adtb shelter.db
adtb> transact
  ...   update a in adopters where a.name == "Ann" set Adopter { a with phone: Some { value: "555-0101" } }
  ...   update b in adopters where b.name == "Bo" set Adopter { b with phone: Some { value: "555-0100" } }
  ...
adtb> from a in adopters select { name: a.name, phone: a.phone } order by a.name
{ name: "Ann", phone: Some { value: "555-0101" } }
{ name: "Bo", phone: Some { value: "555-0100" } }
{ name: "Cy", phone: None }
adtb> \q
```

A block can name what it computes with `let` and give a result with
`return`. `let p = insert ...` binds the inserted row, so the next
item can refer to it with `key p`:

```console
$ adtb shelter.db
adtb> transact
  ...   let p = insert pets.Rabbit { name: "hop", age: 2, size: Small, diet: Normal }
  ...   let a = insert adoptions { pet: key p, adopter: adopters.key("cy@example.com"), fee: 15.00 }
  ...   return { pet: p.name, pets_before: count(x for x in pets) }
  ...
{ pet: "hop", pets_before: 3 }
adtb> \q
```

Every query in a block reads the database as it was when the block
began, so `count(x for x in pets)` does not count `hop`: the rows the
block writes are reached only through the names `let` gives them. The
block's writes all happen at its end.

`fail` stops a block with a message of your own, and `if ... then ...
else ...` chooses between writes, with `skip` as the write that does
nothing:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> transact
  ...   let n = count(a for a in adoptions where a.adopter == adopters.key("cy@example.com"))
  ...   if n > 0 then fail "Cy has a pet already" else skip
  ...
Cy has a pet already
adtb> \q
```

When any item fails, nothing the block did is applied.

## Several texts at once

A text given to `-c`, or typed at the prompt on one line, may hold
several writes one after another. They run in order as one
transaction, each its own statement, and the first that fails undoes
them all:

<!-- exit 1 0 -->
```console
$ adtb shelter.db -c 'insert pets.Dog { name: "fido", age: 2, size: Medium, diet: Normal, breed: "pug" } insert adoptions { pet: pets.key("fido"), adopter: adopters.key("bo@example.com"), fee: 25.00 } insert adoptions { pet: pets.key("fido"), adopter: adopters.key("ann@example.com"), fee: 25.00 }'
`shelter::adoptions` already holds a row with key `adoptions.key(pets.key("fido"))`
$ adtb shelter.db -c 'from p in pets where p.name == "fido" select p'
```

Unlike the items of a block, each of these writes is a statement: the
second sees the dog the first inserted, and the constraints are
checked after each.

## Interactive transactions

At the prompt, `\begin` opens a transaction that the texts after it
run in, `\commit` makes their changes permanent and `\rollback`
discards them. Inside it each text sees the changes of the ones
before:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> \begin
adtb> insert pets.Dog { name: "fido", age: 2, size: Medium, diet: Normal, breed: "pug" }
adtb> insert adoptions { pet: pets.key("fido"), adopter: adopters.key("bo@example.com"), fee: 25.00 }
adtb> from a in adoptions select a.pet.name
"fido"
"hop"
"tom"
adtb> insert pets.Rabbit { name: "old", age: 50, size: Small, diet: Normal }
constraint `shelter::sensible_age` is violated by 1 row
adtb> \commit
adtb> from a in adoptions select a.pet.name
"fido"
"hop"
"tom"
adtb> \q
```

A text that fails inside the transaction prints its error and undoes
only itself; the transaction stays open with what the earlier texts
did, and `\commit` keeps it. The constraints are checked after every
text, never deferred to the commit, so the phone swap above still has
to be one block.

If you leave the shell with a transaction open, it is rolled back:

```console
$ adtb shelter.db
adtb> \begin
adtb> delete w in walks
adtb> \q
the open transaction is rolled back
```

## When transactions conflict

Two transactions that run at the same time and could not have run one
after the other with the same results cannot both commit: one fails
with a serialization failure, and running it again is the fix. Text
that writes is rerun by the engine on such a failure, up to the
`RetryLimit` setting, five times unless you change it. A `transact`
block can set its own limit with `retry`:

```console
$ adtb shelter.db
adtb> transact retry 3
  ...   insert adopters { email: "dee@example.com", name: "Dee", phone: None }
  ...   return count(a for a in adopters)
  ...
3
adtb> \q
```

The shell runs one transaction at a time, and one process owns a
database file, so you will not see a conflict from the shell. They
arise between threads of one application. The rules are in
[the specification](../../spec/transactions.md).

[Page 7](7-migrations.md) changes the schema of a database that holds
data.

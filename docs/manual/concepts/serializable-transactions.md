# Serializable transactions

Every transaction in adtb is serializable: the committed transactions
have the effect of running one at a time, in some order, each against
the state the ones before it left. There is no other isolation level
and no setting that selects one, so there are no anomalies to reason
about level by level. If a program is correct when its transactions
run alone, it is correct when they run together.

The engine provides this without locks that readers wait on. A read
never waits for a write and a write never waits for a read. When two
transactions conflict in a way no serial order could explain, one of
them is aborted with a serialization failure and can be run again.

## Statements and constraints

A statement is one write, or one `transact` block, applied as a single
change. Constraints hold at every statement boundary: a statement that
would leave the database violating one is refused as a whole, and the
database is as it was before it. Save this as `bank.adt`:

```adt bank.adt
module bank

type Account = { name: Text, balance: I64 }
relation accounts : Account key name
check not_overdrawn on accounts (balance >= 0)
```

```console
$ adtb --create --schema bank.adt bank.db
adtb> insert accounts { name: "ann", balance: 100 }
adtb> insert accounts { name: "bo", balance: 0 }
adtb> \q
```

Text given to the shell with `-c` runs as one transaction. When it
holds several items, each is a statement, and the first that fails
ends the text with nothing applied. This transfer would overdraw Ann,
so neither update happens:

```console
$ adtb bank.db -c 'update a in accounts where a.name == "bo" set { a with balance: a.balance + 500 }
update a in accounts where a.name == "ann" set { a with balance: a.balance - 500 }'
constraint `bank::not_overdrawn` is violated by 1 row
$ adtb bank.db -c 'from a in accounts select a order by a.name'
Account { name: "ann", balance: 100 }
Account { name: "bo", balance: 0 }
```

## Transact blocks

A `transact` block is one statement made of several items. Every query
and write in it reads the state before the block, not the effects of
the items above it, and the block's writes are applied together at its
end. A name bound with `let` carries a value from one item to the
next, and `return` gives the block's result:

```console
$ adtb bank.db
adtb> transact
  ...   update a in accounts where a.name == "ann" set { a with balance: a.balance - 30 }
  ...   update a in accounts where a.name == "bo" set { a with balance: a.balance + 30 }
  ...   let before = sum(a.balance for a in accounts)
  ...   let ann = count(a for a in accounts where a.balance == 100)
  ...   return { before: before, ann_still_100: ann }
  ...
{ ann_still_100: 1, before: 100 }
adtb> from a in accounts select a order by a.name
Account { name: "ann", balance: 70 }
Account { name: "bo", balance: 30 }
adtb> \q
```

Inside the block Ann still has 100: the update above the `let` has not
been applied yet. Because every item reads the same state, two updates
of one row in one block do not run one after the other. They propose
two new values for one key, and the block is refused:

```console
$ adtb bank.db
adtb> transact
  ...   update a in accounts where a.name == "ann" set { a with balance: a.balance - 10 }
  ...   update a in accounts where a.name == "ann" set { a with balance: a.balance - 20 }
  ...
`bank::accounts` already holds a row with key `accounts.key("ann")`
adtb> transact
  ...   update a in accounts where a.name == "ann" set { a with balance: 0 }
  ...   fail "stop here"
  ...
stop here
adtb> from a in accounts select a.balance order by a.balance
30
70
adtb> \q
```

`fail` ends a block with an error of your own, and nothing it did is
applied. A block can also be `read_only`, which the engine checks:

```console
$ adtb bank.db -c 'transact read_only
  insert accounts { name: "cy", balance: 1 }'
a `read_only` block may not write
```

## Interactive transactions

In the shell, `\begin` opens a transaction that the following texts run
in, one statement each, until `\commit` or `\rollback`. A text that
fails is undone and the transaction stays open, so you can go on or
roll back:

```console
$ adtb bank.db
adtb> \begin
adtb> update a in accounts where a.name == "ann" set { a with balance: a.balance - 50 }
adtb> from a in accounts select a order by a.name
Account { name: "ann", balance: 20 }
Account { name: "bo", balance: 30 }
adtb> update a in accounts where a.name == "bo" set { a with balance: a.balance - 100 }
constraint `bank::not_overdrawn` is violated by 1 row
adtb> \commit
adtb> from a in accounts select a order by a.name
Account { name: "ann", balance: 20 }
Account { name: "bo", balance: 30 }
adtb> \begin
adtb> delete a in accounts
adtb> \q
the open transaction is rolled back
```

Leaving the shell with a transaction open rolls it back.

## Concurrency and retry

One process owns a database file; a second shell on the same file is
refused while the first has it open:

```text
bank.db: the file is locked by another process
```

Concurrent transactions come from the threads of the program that
embeds the engine, through the in-process API. Under serializability a
conflict between them does not produce a wrong answer: it aborts one
transaction with a serialization failure, which is safe to retry from
the start. A `transact` block can ask the engine to do that itself, as
`transact retry 3`:

```console
$ adtb bank.db -c 'transact retry 3
  update a in accounts where a.name == "bo" set { a with balance: a.balance - 10 }
  update a in accounts where a.name == "ann" set { a with balance: a.balance + 10 }
  return sum(a.balance for a in accounts)'
50
```

The shell runs one transaction at a time, so it never shows a
serialization failure itself.

The rules are in [transactions](../../spec/transactions.md#1-serializability)
and [statements](../../spec/transactions.md#2-statements).

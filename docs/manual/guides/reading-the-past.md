# Reading the past

adtb can read the database as it was at an earlier commit. An `at
commit` or `at instant` block evaluates a query at a past state, and
its result is a set you can use beside reads of the current state.
This guide keeps a window of history, lists the commits in it, reads
them, and compares them with now. The full rules are in
[reading the past](../../spec/derived.md#6-reading-the-past).

Save this schema as `shelter.adt`:

```adt shelter.adt
module shelter

type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
relation pets : Pet key name
```

## Keep history

By default the engine keeps no row version that no running
transaction can see, so there is no past to read. The file setting
`RetentionWindow` says how far back reads may reach: the engine keeps
every version a state within that length of now can see. Set it before
the commits you will want to read; a commit made while the window was
shorter is not kept. The setting is stored in the file.

```console
$ adtb --create --schema shelter.adt shelter.db
adtb> set setting RetentionWindow = { length: 7d }
RetentionWindow { id: settings.key(470), length: 7d }
adtb> insert pets.Cat { name: "tom", lives: 9 }
adtb> insert pets.Dog { name: "rex", good: true }
adtb> update c in pets.Cat where c.name == "tom" set Cat { c with lives: 8 }
adtb> count(c for c in sys::commit_times)
3
adtb> \q
```

`sys::commit_times` lists every commit inside the window: its
`position`, a `Position { epoch, lsn }` naming its place in the log,
and `at`, the instant it committed. Each of the three writes above is
one commit. A command such as `set setting` is not a transaction and
has no commit of its own.

## Read a past state

`at commit p { q }` evaluates query `q` at the latest safe snapshot at
or before position `p`, and `at instant t { q }` at the latest one at
or before the last commit at or before instant `t`.
`sys::safe_snapshots` lists the safe snapshots. Here every commit is
one, so each block reads the state just after a commit. The block is
an expression of set type, so it can be bound in a `from` like a
relation:

```console
$ adtb shelter.db
adtb> from c in sys::commit_times select { pets: count(p for p in at commit c.position { from p in pets select p }), lives: at commit c.position { from x in pets.Cat select x.lives } }
{ lives: [8], pets: 2 }
{ lives: [9], pets: 1 }
{ lives: [9], pets: 2 }
adtb> from c in sys::commit_times select at instant c.at { from p in pets select p.name }
["rex", "tom"]
["tom"]
adtb> \q
```

The first query gives one row per commit: after the first, one pet
with nine lives; after the second, two pets; after the third, tom has
eight. The second query reads each commit by its instant. Two of the
three states have the same names, and a result is a set, so they are
one element.

## Compare with now

A past read and a read of the current state can meet in one query.
This one finds every cat whose lives changed since some commit in the
window, with the old and the current count:

```console
$ adtb shelter.db -c 'from c in sys::commit_times, old in at commit c.position { from x in pets.Cat select x }, cur in pets.Cat where old.name == cur.name and old.lives != cur.lives select { name: cur.name, was: old.lives, now: cur.lives }'
{ name: "tom", now: 8, was: 9 }
```

A row read in an `at` block is a value, not a row of a relation, so
`key old` is refused; compare the key fields instead, as `old.name ==
cur.name` does.

## Inside a transaction

An `at` block reads only committed states. Inside a `transact` block
it does not see the block's own writes, and a past read never takes
part in a serialization conflict:

```console
$ adtb shelter.db
adtb> transact
  ...   update c in pets.Cat where c.name == "tom" set Cat { c with lives: 7 }
  ...   return from c in sys::commit_times select at commit c.position { from x in pets.Cat select x.lives }
[8]
[9]
adtb> from c in pets.Cat select c.lives
7
adtb> \q
```

## When the past is not available

A read the engine cannot serve is refused with `HistoryUnavailable`,
and the message names the oldest position it keeps: "no history is
kept before { epoch: E, lsn: N }". This happens when:

- the position is older than the retention window, or than the
  moment the window was raised;
- the position is later than the reading transaction's snapshot;
- the block reads a relation that did not exist at that position;
- in this engine, the block reads a relation that a later migration
  rebuilt, such as by adding a field to its type. A migration that
  leaves the relation alone, such as one adding another relation,
  keeps its history readable.

Lowering the window drops what it no longer covers. At `0s`,
`sys::commit_times` is empty:

```console
$ adtb shelter.db
adtb> set setting RetentionWindow = { length: 0s }
RetentionWindow { id: settings.key(470), length: 0s }
adtb> count(c for c in sys::commit_times)
0
adtb> \q
```

See [errors](../reference/errors.md#historyunavailable).

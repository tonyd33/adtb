# Writing constraints

A constraint states something that must always be true of the data,
and the engine refuses any statement that would make it false. This
guide shows each kind of constraint the engine runs, how to read the
message when a write breaks one, and how to write a constraint the
engine will accept in the first place. The full rules are in
[constraints](../../spec/constraints.md).

The running example books meeting rooms. A booking is held for an
hour, or cancelled with a reason. Save this as `rooms.adt`:

```adt rooms.adt
module rooms

type Room = { name: Text, seats: I64 }
relation rooms : Room key name

type Booking { id: I64, room: Key rooms on delete cascade, by: Text }
  = Held { hour: I64, people: I64 }
  | Cancelled { reason: Text }
relation bookings : Booking key id
  unique (room, hour) in Held
  index (room)

check sensible on rooms (seats > 0)
check office_hours on bookings.Held (hour >= 8 and hour < 18)

constraint fits =
  from b in bookings.Held
  where b.people > b.room.seats
  select b

constraint no_uncancel =
  from o in deleted(bookings.Cancelled), n in inserted(bookings.Held)
  where key o == key n
  select n
```

```console
$ adtb --create --schema rooms.adt rooms.db
adtb> insert rooms { name: "attic", seats: 4 }
adtb> insert bookings.Held { id: 1, room: rooms.key("attic"), by: "ann", hour: 9, people: 3 }
adtb> \q
```

## A constraint is a query that must return nothing

Every constraint in adtb has one meaning: a query whose result must be
empty. `fits` selects the bookings with more people than their room
has seats; the database is valid when there are none. You write the
query for the bad rows, and the engine keeps that set empty. Keys,
uniques, references and checks are shorter ways to write particular
queries of this kind.

## The kinds you can declare

Each kind below is broken once in this session, and each message
names the constraint that refused the write:

<!-- exit 1 -->
```console
$ adtb rooms.db
adtb> insert rooms { name: "hall", seats: 0 }
constraint `rooms::sensible` is violated by 1 row
adtb> insert bookings.Held { id: 2, room: rooms.key("attic"), by: "bo", hour: 9, people: 2 }
constraint `rooms::bookings.unique(room, hour) in Held` is violated by 2 rows
adtb> insert bookings.Held { id: 3, room: rooms.key("attic"), by: "bo", hour: 20, people: 2 }
constraint `rooms::office_hours` is violated by 1 row
adtb> insert bookings.Held { id: 4, room: rooms.key("attic"), by: "bo", hour: 10, people: 9 }
constraint `rooms::fits` is violated by 1 row
adtb> insert bookings.Held { id: 5, room: rooms.key("cellar"), by: "bo", hour: 10, people: 1 }
fault `NoSuchRow`
adtb> insert bookings.Held { id: 1, room: rooms.key("attic"), by: "cy", hour: 11, people: 1 }
`rooms::bookings` already holds a row with key `bookings.key(1)`
adtb> \q
```

- A check, `check NAME on RELATION (EXPR)`, tests each row on its own.
  Inside the parentheses the row's fields are in scope by name.
  `office_hours` is declared on `bookings.Held`, so it reads `hour`,
  which only held bookings have.
- A unique, a clause of the relation, says no two rows share the listed
  fields. `in Held` scopes it to one constructor: two cancelled
  bookings of the same room and hour are fine. The message counts the
  rows involved, both of them.
- A reference, any `Key` field, says the row it names exists. Its
  constraint is named after the field, `rooms::bookings.room`, and a
  dangling reference is reported as "constraint
  `rooms::bookings.room` is violated by 1 row". Here, though, the
  booking for the missing room `cellar` fails differently: `fits`
  follows `b.room` to read its seats, and following a reference to a
  row that does not exist is a fault, `NoSuchRow`. When checking a
  constraint faults, the statement fails with the fault.
- A key says no two rows share it. A collision is reported as the key
  it repeats rather than as a constraint.
- A constraint, `constraint NAME = QUERY`, is any query that must
  return nothing.

A unique scoped to a constructor lets a cancelled booking free its
slot:

```console
$ adtb rooms.db
adtb> update b in bookings where b.id == 1 set Cancelled { b with reason: "ill" }
adtb> insert bookings.Held { id: 2, room: rooms.key("attic"), by: "bo", hour: 9, people: 2 }
adtb> from b in bookings select b order by b.id
Cancelled { id: 1, room: rooms.key("attic"), by: "ann", reason: "ill" }
Held { id: 2, room: rooms.key("attic"), by: "bo", hour: 9, people: 2 }
adtb> \q
```

## Constraints on change

Some rules are about how data may change, not how it may be. Inside a
constraint, `inserted(R)` is the rows a statement inserts into `R`,
`deleted(R)` the rows it deletes, as they were before, and `prior(R)`
all of `R` before the statement. An update shows up as its old row in
`deleted` and its new row in `inserted`, with the same key; a retag
from one constructor to another shows up in `deleted` of the first and
`inserted` of the second.

`no_uncancel` selects bookings retagged from `Cancelled` back to
`Held`, so a cancelled booking stays cancelled:

<!-- exit 1 -->
```console
$ adtb rooms.db -c 'update b in bookings where b.id == 1 set Held { b with hour: 15, people: 3 }'
constraint `rooms::no_uncancel` is violated by 1 row
```

## Delete actions

A reference field also says what deleting its target does to the row
that holds it, with `on delete`:

- `restrict`, the default when you write nothing: the delete fails
  while a row still refers to the target.
- `cascade`: the referring row is deleted too, in the same statement.
- `clear`, on an `Option (Key ...)` field: the field becomes `None`.
- `retag C { ... }`: the referring row moves to constructor `C` of
  its own sum, which must not have the field.

`bookings.room` cascades, so deleting the room deletes its bookings:

```console
$ adtb rooms.db
adtb> delete r in rooms where r.name == "attic"
adtb> from b in bookings select b.id
adtb> \q
```

The full rules, including when actions run and how they combine, are
in [delete actions](../../spec/writes.md#7-delete-actions).

## When constraints are checked

Constraints hold at the end of every statement, never in the middle
of one. A `transact` block is one statement, so its writes may pass
through states that break a constraint as long as the end state does
not. Here a booking names a room the same block inserts after it:

```console
$ adtb rooms.db
adtb> transact
  ...   insert bookings.Held { id: 7, room: rooms.key("loft"), by: "ann", hour: 9, people: 2 }
  ...   insert rooms { name: "loft", seats: 6 }
  ...
adtb> from b in bookings select { id: b.id, room: b.room.name }
{ id: 7, room: "loft" }
adtb> \q
```

A statement that breaks a constraint applies nothing: no part of it
is kept. Checking never waits for commit, so an interactive
transaction sees the refusal at the statement that caused it.

## Write constraints the engine admits

The engine checks a constraint incrementally: for each statement it
looks only at the rows the statement changed, and reaches the rest of
the database through indexes. A constraint is admitted only when such
a check exists, so its cost does not grow with the size of the
database. When it does not, the migration that adds it is refused,
and the refusal names what is missing.

This constraint limits each person to three held bookings. Counting a
person's bookings needs an index on `by`, which `bookings` does not
have:

```adt busy.adt
module rooms
add constraint three_each =
  from b in bookings.Held
  where count(x for x in bookings.Held where x.by == b.by) > 3
  select b
```

<!-- exit 1 -->
```console
$ adtb --schema rooms.adt --schema busy.adt rooms.db -c 'from r in rooms select r.name'
rooms.db: migration 2 is refused: constraint rooms::three_each is not admitted: changes to rooms::bookings.Held: index (by) in rooms::bookings.Held is missing
```

Add the index in the same migration and it is admitted:

```adt busy-indexed.adt
module rooms
add index (by) in bookings.Held
add constraint three_each =
  from b in bookings.Held
  where count(x for x in bookings.Held where x.by == b.by) > 3
  select b
```

```console
$ adtb --schema rooms.adt --schema busy-indexed.adt rooms.db -c 'from r in rooms select r.name'
"loft"
```

Within that bound a constraint may join and filter, use `exists` and
`not exists`, and use `count`, `sum`, `min` and `max` over rows that
share the leading fields of an index. Recursion with `fix` is outside
it. `inserted` and `deleted` may not be read under `not` or inside an
aggregate:

```adt once.adt
module rooms
add constraint once =
  from r in rooms
  where not exists d in inserted(rooms) where key d == key r
  select r
```

<!-- exit 1 -->
```console
$ adtb --schema rooms.adt --schema busy-indexed.adt --schema once.adt rooms.db -c '1'
rooms.db: migration 3 is refused: constraint rooms::once is not admitted: `inserted(rooms::rooms)` is read under negation
```

## Adding a constraint to existing data

A constraint added by migration must already hold. If it does not, the
migration is refused and the message shows a row that breaks it:

```adt big.adt
module rooms
add check big on rooms (seats > 10)
```

<!-- exit 1 -->
```console
$ adtb --schema rooms.adt --schema busy-indexed.adt --schema big.adt rooms.db -c '1'
rooms.db: migration 3 is refused: constraint rooms::big is violated by Room { name: "loft", seats: 6 }
```

Repair the data first, with ordinary writes, then apply the
migration. [Evolving a schema](evolving-a-schema.md) covers the other
refusals a migration can meet.

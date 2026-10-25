---
title: What a key may be made of
pick: any
options:
  composite:
    title: A key may span several fields
  sum-valued-key:
    title: A key field may have a value-level sum type
  ref-in-key:
    title: A key may include a reference to another relation
  record-key:
    title: A key field may have a record type
---
Base keys over one scalar field are assumed. These widen what a key
index must compare and order.

## model/composite

```
type Membership = { user: Key users, team: Key teams, role: Role }
relation memberships : Membership key (user, team)
```

## model/sum-valued-key

```
type Login = { provider: Google | Facebook, subject: Text }
relation logins : Login key (provider, subject)
```

Needs a total order on sum values for the index.

## model/ref-in-key

A weak entity: `type Line = { order: Key orders, n: Int, ... }` with
`relation lines : Line key (order, n)`. Under `model/typed-key` the key
holds the order's key.

## model/record-key

`key (span)` where `span: { from: Date, to: Date }`; equal to a composite
key over the record's fields.

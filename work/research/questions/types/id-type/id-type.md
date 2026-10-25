---
title: The type of a generated key field
pick: one
raised-by: [model/declared-key]
options:
  key-type:
    title: The field has the same Key type its referrers hold
  newtype:
    title: The user declares an id newtype over the representation
  raw:
    title: The field is a plain I64 or Uuid
---
A reference is the target key typed by its constructor set
(`model/typed-key`): `Key users`. Whether the key field itself has
that type decides whether `u.id` can be passed where a `Key users`
is expected without a conversion, and whether a `Key orders` can be
mistaken for it.

## types/id-type/key-type

```
type User = { id: Key users = generate uuidv7, email: Email }
from s in sessions where s.user == u.id select s
```

The key field and every reference to the row share one type; its
representation is the generator's.

## types/id-type/newtype

```
type UserId = new Uuid
type User = { id: UserId = generate uuidv7, ... }
```

`model/newtype`: a domain is a distinct nominal type. A reference
`Key users` then wraps a `UserId`.

## types/id-type/raw

`id: I64 = generate sequence`; `Key users` and `Key orders` are still
distinct types, the key fields are not.

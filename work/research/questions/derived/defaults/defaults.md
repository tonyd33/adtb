---
title: Where field defaults attach
pick: one
options:
  none:
    title: No defaults; an insert supplies every field
  on-type:
    title: Defaults on a type's fields, applied wherever a value is constructed
  on-relation:
    title: Defaults on a relation variable, applied by inserts into it
    links:
      - {rel: gains, to: surface/extent/separate}
---
Types and the relation variables holding them are distinct
(`surface/extent/separate`). A default is either part of a value's
construction or part of an insert. Each constructor is a record, so a
default is naturally per constructor.

## derived/defaults/none

```
fn new_session (u: Key users) -> Session = { user: u, expires: tx_time() + 30d }
insert sessions (new_session(u))
```

Defaults are schema functions; generated types in host languages
(`surface/codegen/schema-types`) supply the rest.

## derived/defaults/on-type

```
type Status = Pending | Active { since: Instant } | Closed { at: Instant }
type Account = { id: Id, status: Status = Pending }
```

`{id: 3}` is an `Account` in any query. A default that reads the clock
or generates a key makes construction impure inside pure queries.

## derived/defaults/on-relation

```
relation sessions : Session
  key id
  default { id: generate, expires: tx_time() + 30d }
relation identities : Identity
  key id
  default { verified: false } in Google
```

Only an insert into `sessions` fills them; a value built elsewhere
gives every field. `tx_time()` is the transaction's time, one value per
transaction, so the change set stays deterministic. Per-constructor
defaults name a constructor after `in`, as uniques do.

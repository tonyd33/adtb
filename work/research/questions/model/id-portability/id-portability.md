---
title: Engine ids do not survive export and merge between databases
pick: any
raised-by: [model/engine-id]
options:
  uuid-ids:
    title: Ids are globally unique so two databases never collide
  id-as-value:
    title: An id is a readable value that import may set
  live:
    title: Live with ids local to one database
    excludes: [model/id-portability/uuid-ids, model/id-portability/id-as-value]
---
A dump loaded into another database, or two databases merged, must keep
references pointing at the same rows.

## model/id-portability/uuid-ids

Gel's `id` is a globally unique uuid
(https://docs.geldata.com/reference/datamodel/objects).

## model/id-portability/id-as-value

`s.identity.id` is a value of type `Key identities`; an import may supply
ids, checked unique. Moves `model/engine-id` toward `model/declared-key`.

## model/id-portability/live

Export rewrites references to natural keys and import resolves them.

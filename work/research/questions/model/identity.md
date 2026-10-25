---
title: How a row gets identity
pick: one
options:
  engine-id:
    title: Every row has an engine identity outside its fields
    excludes: [model/set-of-type]
    links:
      - {rel: prior-art, to: prior/gel}
      - {rel: prior-art, to: prior/datomic}
      - {rel: prior-art, to: prior/apg}
  declared-key:
    title: Identity is a declared key over fields and a key field may be engine-generated
    links:
      - {rel: prior-art, to: prior/third-manifesto}
  content-hash:
    title: Identity is a hash of the fields the row was constructed from
    links:
      - {rel: prior-art, to: prior/model-rel}
---
What makes a row the row a reference points at.

## model/engine-id

Every row of every relation has an id the engine assigns, not a field
of its type. Two rows may agree on every field. Natural keys are
ordinary `unique` constraints.

```
type Session = { identity: Key identities, expires: Time }
relation sessions : Session
relation identities : Identity
  unique (gid) in Google
```

Gel: every object has a required, readonly, globally unique `id: uuid`
(https://docs.geldata.com/reference/datamodel/objects). Datomic entity
ids (https://docs.datomic.com/schema/schema-reference.html). APG
elements have identity apart from their value (arXiv 1909.04881 sec. 3.1).
Excluded by `model/set-of-type`: a set of values cannot hold two equal
values.

## model/declared-key

Every relation declares at least one key over its fields. A field may
take a generated default, so a surrogate is a field like any other,
visible and settable on import.

```
type Identity { id: Id = generate } = EmailPass { ... } | ...
relation identities : Identity key id
type Country = { code: Text, name: Text }
relation countries : Country key code
```

Third Manifesto RM Prescription 15 (every relvar has a candidate key) and
OO Proscription 2 (no object IDs),
https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf.

## model/content-hash

The id is a hash of the type name and the values given at construction,
then detached from them: a later update to those fields keeps the id.
Inserting the same construction twice yields the same row.

```
entity Identity.Google from (gid)
```

RelationalAI Rel entity types
(https://rel.relational.ai/rel/ref/data-types/entity).

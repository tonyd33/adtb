---
title: How a reference to a relation-level sum is written
pick: one
raised-by: [fk/to-ctor]
options:
  ref-type:
    title: A reference type naming a constructor set
    links:
      - {rel: prior-art, to: prior/gel}
  key-fk:
    title: Fields holding key values, with a references constraint
---
Whether a reference is a value of its own type, or ordinary key fields
the engine constrains.

## surface/ref/ref-type

```
type Session = Active { who: Key identities.Google, ... } | ...
relation sessions : Session key id

type Owner = UserOwner { u: Key users } | OrgOwner { o: Key orgs }
relation owners : Owner key id

from s in sessions.Active select s.who.gid
```

A reference names a constructor set of a relation variable, never of a
type, since one type may have several relation variables. Navigation
follows the reference; the key's fields are not restated. An EER
category is a relation whose constructors each hold one reference
(`owners`); a reference inside a value-level field is excluded by
`model/refs-top-level`. Gel links.

## surface/ref/key-fk

```
entity Session key id = Active { who_id: Uuid references Identity.Google(id) } | ...

from s in Session.Active, g in Identity.Google
where s.who_id == g.id select g.gid
```

The relational form: references are values of the key's type joined
explicitly. Natural keys are visible and comparable.

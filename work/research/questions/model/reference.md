---
title: Whether references are values or constraints on key fields
pick: one
raised-by: [fk/to-sum, fk/to-ctor, fk/to-subset]
options:
  ref-type:
    title: A reference is an opaque value of type ref S for a constructor set S
    links:
      - {rel: prior-art, to: prior/gel}
      - {rel: prior-art, to: prior/apg}
      - {rel: prior-art, to: prior/lang-machiavelli}
  fk-on-field:
    title: A field holds the target key value and an FK constraint is declared on it
    links:
      - {rel: prior-art, to: prior/third-manifesto}
  typed-key:
    title: A field holds the target key as a visible value whose type names the target constructor set
---
The accepted FK targets are a sum, a constructor or a constructor set.
How a referrer spells one.

## model/ref-type

```
type Session    = { identity: ref identities, expires: Time }
type Reset      = { identity: ref identities.EmailPass, token: Bytes }
type OAuthToken = { identity: ref identities.(Google | Facebook) }

from s in sessions select s.identity.user
from r in resets select r.identity.email     -- typed by EmailPass
```

The type is the FK; dereference is a path. The value has equality and
nothing else observable. Gel links
(https://docs.geldata.com/reference/datamodel/links), APG `Lbl l`
(arXiv 1909.04881 sec. 3), Machiavelli `ref(τ)` (Buneman, Ohori, TODS
1996, https://homepages.inf.ed.ac.uk/opb/papers/TODS1996.pdf).

## model/fk-on-field

```
type Reset = { identity_id: Id, token: Bytes }
relation resets : Reset key token
  fk (identity_id) -> identities.EmailPass
from r in resets, e in identities.EmailPass where r.identity_id == e.id select e.email
```

Navigation is a join. The field's type says nothing about the target.

## model/typed-key

```
type Reset = { identity: Key identities.EmailPass, token: Bytes }
relation resets : Reset key token
from r in resets select r.identity.email
print r.identity           -- 42
```

`Key S` wraps the key type of S: readable, printable, constructible from
a literal (checked on write), and dereferenceable. Datomic refs are
entity ids readable as numbers, untyped
(https://docs.datomic.com/schema/schema-reference.html).

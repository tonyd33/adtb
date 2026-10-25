---
title: What a relation declaration is
pick: one
options:
  set-of-type:
    title: A relation is a variable holding a set of values of a declared type
    links:
      - {rel: prior-art, to: prior/third-manifesto}
      - {rel: prior-art, to: prior/lang-nrc}
  entity-decl:
    title: A relation is an entity-set declaration with its own fields
    links:
      - {rel: prior-art, to: prior/gel}
      - {rel: prior-art, to: prior/apg}
      - {rel: prior-art, to: prior/eer}
---
Whether types and stored relations are separate concepts. Under the first,
the level a sum sits at is only where its type is used; under the second,
an entity sum and a value sum are declared by different forms.

## model/set-of-type

Types are declared once; a relation is a named variable of type
`Set T` with keys. A relation-level sum is a relation whose element type
is a sum; its constructor relations are the restrictions to each tag.

```
type Identity = EmailPass { email: Email, hash: Bytes }
              | Google    { gid: Text }
relation identities : Identity key id

from g in identities.Google select g.gid
```

The same `Identity` type may also be a field type elsewhere. Third
Manifesto relvars (`VAR r BASE RELATION {...} KEY {...}`, TTM RM
Prescriptions 6, 7, 15, https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf)
and NRC `Set T` (Buneman et al., TCS 149(1), 1995).

## model/entity-decl

The declaration is the entity set; there is no separate element type.
Value types are declared separately.

```
entity Identity = EmailPass { email: Email, hash: Bytes }
                | Google    { gid: Text }
type Address = US { zip: Text } | Intl { country: Text, postcode: Text }
```

Gel `type User { ... }` is an object type and its set at once
(https://docs.geldata.com/reference/datamodel/objects); APG labels carry
both a type and an element set (arXiv 1909.04881 sec. 3).

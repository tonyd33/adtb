---
title: Overlapping specializations need an encoding without overlap
pick: some
raised-by: [sum/disjoint-total, sum/disjoint-partial]
options:
  facets-1n:
    title: Entity 1:N a sum of facets, unique (entity, tag) when one per kind
  facets-optional:
    title: One optional 1:1 relation per facet
  live:
    title: Live without overlap
    excludes: [sum/overlap/facets-1n, sum/overlap/facets-optional]
---
EER's overlapping PART = MANUFACTURED and PURCHASED, or a user with a
password and a Google login, cannot be one row of one constructor.

## sum/overlap/facets-1n

```
type Identity { user: Key users } = EmailPass { ... } | Google { ... }
relation identities : Identity
  key id
  unique (user) in EmailPass
  unique (user) in Google
```

Per-kind uniqueness is one unique per constructor.

## sum/overlap/facets-optional

`relation manufactured : { part: Key parts, ... } key part`, and
`purchased` alike.

## sum/overlap/live

Overlap is out of scope.

---
title: What a group by constructor groups on
pick: one
raised-by: [query/agg-scope/whole]
options:
  tag-type:
    title: Each sum has a derived tag type with one nullary constructor per constructor
  match-key:
    title: The key is a match returning a user-chosen value
  per-relation:
    title: One aggregate per constructor relation, combined by union
---
"Per-constructor aggregates are group by ctor" needs a key value naming
the constructor.

## lang/group/ctor-key/tag-type

```
-- derived: type Identity.Tag = EmailPass | Google
from i in identities group i by tag(i) into g select { tag: g.key, n: count(x for x in g) }
```

A sum of nullary constructors, matched exhaustively like any other.
Adding a constructor to `Identity` adds one to `Identity.Tag`. Rust
`std::mem::discriminant` is the untyped analogue.

## lang/group/ctor-key/match-key

```
from i in identities
group i by (match i | EmailPass _ -> "email" | Google _ -> "google") into g
select { kind: g.key, n: count(x for x in g) }
```

No new type; the match keeps it exhaustive.

## lang/group/ctor-key/per-relation

```
{ ctor: "EmailPass", n: count(e for e in identities.EmailPass) }
union { ctor: "Google", n: count(g for g in identities.Google) }
```

Uses `query/ctor-relations` directly; nothing checks that every
constructor appears.

---
title: Smallest unit a relation grant names
pick: one
raised-by: [security/subject/relations]
options:
  sum:
    title: A whole relation variable, every constructor at once
  ctor-set:
    title: Any constructor set of a relation, the same sets indexes and FKs range over
    links:
      - {rel: variant-of, to: index/ctor-set}
---
Constructors are relations, so a grant can stop at a sum or reach into
it.

## security/grain/sum

```
grant read identities to analyst
```

As a table grant.

## security/grain/ctor-set

```
grant read identities.(EmailPass | Google) to support
grant read identities to analyst   -- every constructor, now and later
```

One mechanism: the whole relation is the set of all its constructors,
one constructor is a singleton set. A grant on a named set follows the
set when a constructor joins it (`surface/ctor-set/growth/declare`).

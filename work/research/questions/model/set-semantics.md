---
title: Whether relations are sets or bags
pick: one
options:
  sets:
    title: Every relation is a set and every query result is a set
    links:
      - {rel: prior-art, to: prior/third-manifesto}
  bags:
    title: Relations and results are bags
  stored-sets:
    title: Stored relations are sets with a key and query results are bags
    links:
      - {rel: prior-art, to: prior/lang-nrc}
---
Whether two equal records can both be in a relation.

## model/sets

A relation is a set of records; projection removes duplicates. Third
Manifesto relations (RM Prescription 10, unverified number), Datalog.

```
from s in sessions select s.identity   -- each identity once
```

## model/bags

SQL multiset semantics; `distinct` is explicit.

## model/stored-sets

Every stored relation has a key so it is a set; query results are bags
with `distinct` explicit. NRC treats sets, bags and lists uniformly as
collection monads, so the choice is per expression.

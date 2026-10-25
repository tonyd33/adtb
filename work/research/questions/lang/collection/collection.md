---
title: What a query result is
pick: one
options:
  sets:
    title: Sets, duplicates never exist
    links:
      - {rel: prior-art, to: prior/third-manifesto}
  bags:
    title: Bags, duplicates kept until removed with distinct
    links:
      - {rel: prior-art, to: prior/lang-links}
  lists:
    title: Lists, every result ordered
    links:
      - {rel: prior-art, to: prior/lang-morel}
  kinded:
    title: Collection kind in the type, Set, Bag and List
    links:
      - {rel: prior-art, to: prior/lang-nrc}
---
Stored constructor relations are sets: every row has a key. The question
is what projections, unions and joins produce. `query/union-all`
already fixes a match as a disjoint union, so arms cannot collide on
identity; projecting away the key is where duplicates appear.

## lang/collection/sets

```
from e in employees select e.dept   -- each dept once
```

Tutorial D and the Third Manifesto: a relation has no duplicates.
Evaluation deduplicates after projection unless the planner proves a key
survives.

## lang/collection/bags

```
from e in employees select e.dept            -- one per employee
distinct (from e in employees select e.dept)
```

Bag semantics is what T-LINQ and query shredding normalise and what
SQL engines execute natively; `distinct` is explicit.

## lang/collection/lists

Every comprehension yields a list in generator order; stored relations
iterate in key order. Morel's `from` produces a list (unverified for
every clause).

## lang/collection/kinded

```
from e in employees select e.dept           : Bag Text
from e in employees select distinct e.dept  : Set Text
from e in employees order e.name select e   : List Employee
```

CPL's `{}`, `{| |}` and `[ ]`: one comprehension form, typed by the
kind of its generators and result. Conversions are explicit operators.

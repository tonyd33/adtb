---
title: How a query expresses recursion
pick: one
options:
  none:
    title: No recursion in the language
  rules:
    title: Recursive relations defined by Datalog rules
    links:
      - {rel: prior-art, to: prior/lang-datalog}
      - {rel: prior-art, to: prior/souffle}
      - {rel: prior-art, to: prior/flix}
  fix:
    title: A fixpoint binder inside the comprehension language
---
Transitive closure over a self-referencing relation (org charts, graphs,
bills of materials) is not expressible in the nonrecursive calculus.

## lang/recursion/none

The client iterates, or the schema stores a closure table.

## lang/recursion/rules

```
reports(a, b) :- Employee(b), b.manager = Some a.
reports(a, c) :- reports(a, b), reports(b, c).
from r in reports where r.a == boss select r.c
```

Rules define named relations used by comprehensions as any other
relation. Semi-naive evaluation; magic sets push the `boss` filter in.

## lang/recursion/fix

```
fix reach = (from f in follows where f.src == me select f.dst)
          union (from r in reach, f in follows where f.src == r select f.dst)
in from u in reach select u
```

SQL:1999 recursive common table expressions, as an expression. Linear
recursion only, unless mutual `fix` is allowed.

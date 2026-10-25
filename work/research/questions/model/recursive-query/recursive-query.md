---
title: Recursive values need recursive queries
pick: any
raised-by: [model/recursive-values]
options:
  fold:
    title: A structural recursion primitive over recursive sums
    links:
      - {rel: prior-art, to: prior/lang-nrc}
  fixpoint:
    title: Recursive queries by fixpoint
    links:
      - {rel: prior-art, to: prior/lang-datalog}
  live:
    title: Live with recursive values readable only to a fixed depth
    excludes: [model/recursive-query/fold, model/recursive-query/fixpoint]
---
A non-recursive `match` sees one layer of `Expr`. Evaluating it or
finding every `Num` in it needs recursion in the language.

## model/recursive-query/fold

```
fold e | Num n -> n.n | Add a -> a.l + a.r
```

NRC restricts structural recursion to well-defined forms (Buneman et
al., TCS 1995).

## model/recursive-query/fixpoint

Datalog-style recursive views; recursion over values is a rule that
unfolds one constructor per step, as Soufflé does.

## model/recursive-query/live

Nested `match` to a depth written in the query.

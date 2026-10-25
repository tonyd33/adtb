---
title: Whether value-level sums may be recursive
pick: one
raised-by: [sum/value]
options:
  recursive-values:
    title: A value-level sum may refer to itself and values are finite trees
    links:
      - {rel: prior-art, to: prior/souffle}
      - {rel: prior-art, to: prior/model-egglog}
  relation-recursion-only:
    title: Value types are non-recursive and recursion goes through references between rows
---
`type List a = Nil | Cons { head: a, tail: List a }`, `Json`, an expression
tree in one field. Relation-level recursion (`Node { parent: Key nodes }`)
exists whenever references do.

## model/recursive-values

```
type Expr = Num { n: Int } | Add { l: Expr, r: Expr }
type Rule = { body: Expr }
relation rules : Rule key id
```

Soufflé ADTs and records may be recursive
(https://souffle-lang.github.io/types); egglog `(datatype Expr ...
(Add Expr Expr))`
(https://egraphs-good.github.io/egglog-tutorial/01-basics.html).

## model/relation-recursion-only

A type's unfolding is finite; trees are rows:

```
type ExprNode = Num { n: Int } | Add { l: Key expr_nodes, r: Key expr_nodes }
relation expr_nodes : ExprNode key id
```

APG has no recursive types without meta-edges (arXiv 1909.04881
sec. 6.2); recursion is through `Lbl` references.

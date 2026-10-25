---
title: Byte encoding of a recursive value
pick: one
raised-by: [model/recursive-values]
options:
  preorder:
    title: Tags and payloads in preorder with no sizes
  sized-preorder:
    title: Preorder with each subtree prefixed by its byte length
  arena:
    title: An array of nodes with children named by index
    links:
      - {rel: prior-art, to: prior/layout-union-encoding}
---
`Expr = Num {n: I64} | Add {l: Expr, r: Expr}` has no bounded width,
so the value is one variable-width field (or a large value).

```
Add(Num 1, Add(Num 2, Num 3))
```

## layout/recursive/preorder

```
01 | 00 <1> | 01 | 00 <2> | 00 <3>        tag Add=01 Num=00, n as varint
```

Smallest; reading `r` decodes all of `l`.

## layout/recursive/sized-preorder

```
01 [len l] 00 <1> [len r] 01 [len] 00 <2> [len] 00 <3>
```

Skip a subtree without decoding it; path access `e.r.r` is O(depth).

## layout/recursive/arena

```
nodes: [0] Num 1  [1] Num 2  [2] Num 3  [3] Add(1,2)  [4] Add(0,3)   root = 4
```

Cap'n Proto-style random access by relative pointer or index; shared
subtrees are possible (a DAG, as egglog hash-conses). Index width per
node adds bytes.

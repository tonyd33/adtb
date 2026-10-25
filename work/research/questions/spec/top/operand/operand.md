---
title: "What the operand of top is"
pick: one
options:
  comprehension:
    title: "The operand is a comprehension; the keys are written over its select scope"
  any-set:
    title: "The operand is any set expression; the keys are written over its elements"
---
Raised while implementing the front end against docs/spec/. The
grammar took any expression as the operand of `top`, while the keys
were said to be evaluated in the scope of the operand's `select`,
which only a comprehension has.

## spec/top/operand/comprehension

The operand of `top` is a comprehension, so its keys are written over
the comprehension's `select` scope:

```ebnf
top_expr = "top" add_e "by" order_keys "(" comprehension ")"
```

```adt
top 3 by (p.created_at desc, key p) (from p in posts select p)
```

## spec/top/operand/any-set

Any set expression is an operand, and the keys need a binder for its
element (`top 3 by p -> p.created_at (posts)` or an implicit `it`).
Adds a binder form found nowhere else.

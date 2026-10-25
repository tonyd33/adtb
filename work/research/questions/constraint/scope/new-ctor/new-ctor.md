---
title: Adding a constructor meets constraints scoped by constructor
pick: any
raised-by: [[constraint/scope/match, evolution/add], [constraint/scope/ctor-set, evolution/add]]
options:
  arm-in-add:
    title: The add operator supplies an arm for every match constraint
    links:
      - {rel: variant-of, to: evolution/breaks/recompile}
  whole-grows:
    title: A scope written as the whole sum grows; named sets do not
  live:
    title: Live with the add refused until constraints are rewritten
    excludes: [constraint/scope/new-ctor/arm-in-add, constraint/scope/new-ctor/whole-grows]
---
`add constructor Identity.Apple` makes every match-scoped constraint
on `identities` non-exhaustive, and leaves open whether a check on
`identities` or a joint exclusion over it now covers `Apple`.

## constraint/scope/new-ctor/arm-in-add

```
add constructor Identity.Apple { sub: Text }
  arm login_valid: Apple a -> length(a.sub) > 0
```

The new arm is checked against `Apple`'s heap, which is empty.

## constraint/scope/new-ctor/whole-grows

`check login_valid on identities (...)` means every constructor
present now or later; `check login_valid on identities.(Google |
Facebook) (...)` means those two. Same rule
an index over the whole sum needs.

## constraint/scope/new-ctor/live

Recompilation reports each constraint; the change waits for a rewrite.

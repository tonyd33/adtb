---
title: A recursive query under bag semantics diverges on cycles
pick: any
raised-by: [[lang/collection/bags, lang/recursion/fix], [lang/collection/bags, lang/recursion/rules]]
options:
  set-fix:
    title: Recursive relations are sets whatever the result kind
  live:
    title: Live with divergence on cyclic data
    excludes: [lang/collection/bag-fix/set-fix]
---
Reachability over a `follows` graph with a cycle derives each path again
on each iteration; a bag fixpoint never stops growing. Set fixpoints
terminate on finite data.

## lang/collection/bag-fix/set-fix

The recursive binder is a set; its uses outside the recursion are bags
like any other relation. Datalog engines are set-based throughout.

## lang/collection/bag-fix/live

Authors bound recursion depth.

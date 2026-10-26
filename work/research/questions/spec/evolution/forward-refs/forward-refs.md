---
title: "What names a type in one migration may see"
pick: one
options:
  later-key:
    title: "Key r in a type may name a relation a later operator of the same migration adds; every other name types against the operators before it"
  in-order:
    title: "Every operator types against the operators before it"
  simultaneous:
    title: "The declaring operators of one migration form one simultaneous schema"
---
Raised by conformance programs that declare a reference cycle, and by
the spec's own example adding a type naming `Key devices` before `add
relation devices`. Applying operators strictly in order refuses both.

## spec/evolution/forward-refs/later-key

A reference type `Key r` in a declaration may name a relation variable
that a later operator of the same migration adds. Every other name
types against the operators before it. A reference cycle is declared
within one migration.

## spec/evolution/forward-refs/in-order

Each operator types against the schema the operators before it
produce. A reference cycle cannot be declared at all, since one side
always names a relation that does not exist yet.

## spec/evolution/forward-refs/simultaneous

Every declaring operator of a migration sees every other, whatever
their order. Other operators still apply in order. Wider than cycles
need: a type could name a type declared later, and the order of
declaring operators would stop meaning anything.

---
title: "Where key (insert ...) may appear"
pick: one
options:
  once:
    title: "Only where evaluated exactly once per statement: under no binder and in no conditional position"
  at-most-once:
    title: "Also in a conditional position, inserting the row only when that position is evaluated"
  anywhere:
    title: "Anywhere a write is allowed as an operand"
---
Raised by the implementability review of docs/spec/. writes §2.2 let
`key (insert ...)` appear as any parenthesised operand, including under
a binder that evaluates it once per row, where how many rows it inserts
and which key it denotes were unstated.

## spec/writes/nested-insert/once

`key (insert ...)` is allowed only where it is evaluated exactly once
per statement: not under a comprehension, an `update ... in`, an upsert
arm or a match arm binder, and not in a conditional position (an `if`
branch, the right operand of `and` or `or`, a match arm). Elsewhere it
is a type error. This may be relaxed later; it is not tightened later.

```adt fragment
transact
  if b then insert badges { owner: key (insert users { name: n }), label: l }   -- type error
  else skip
```

## spec/writes/nested-insert/at-most-once

A conditional position is allowed, and the row is inserted only when
the position is evaluated. Whether the statement inserts a row then
depends on a value, and whether the right operand of `and` or `or`
is evaluated decides whether a row is written.

## spec/writes/nested-insert/anywhere

Allowed under any binder, inserting one row per evaluation. Needs a
rule for how many rows and which key each use denotes, and makes a
write's size depend on a query.

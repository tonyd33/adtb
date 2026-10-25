---
title: "Where key (insert ...) may appear"
pick: one
options:
  once:
    title: "Only where evaluated once per statement; not under a comprehension, update in, upsert arm or match arm binder"
  anywhere:
    title: "Anywhere a write is allowed as an operand"
---
Raised by the implementability review of docs/spec/. writes §2.2 let
`key (insert ...)` appear as any parenthesised operand, including under
a binder that evaluates it once per row, where how many rows it inserts
and which key it denotes were unstated.

## spec/writes/nested-insert/once

`key (insert ...)` is allowed only where it is evaluated once per
statement: not under a comprehension, an `update ... in`, an upsert arm
or a match arm binder. Elsewhere it is a type error. This may be relaxed
later; it is not tightened later.

## spec/writes/nested-insert/anywhere

Allowed under any binder, inserting one row per evaluation. Needs a
rule for how many rows and which key each use denotes, and makes a
write's size depend on a query.

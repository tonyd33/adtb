---
title: "How the element types of fix binders are found"
pick: one
options:
  from-base:
    title: "Inferred from the union operands that read no untyped binder, repeated until every binder is typed"
  annotated:
    title: "Each fix binder's type is written"
  join-iterate:
    title: "Iterate joins over every operand to a fixpoint"
  least:
    title: "Each binder's element type is the least type at which every body checks against Set of it"
---
Raised while implementing the front end against docs/spec/. A `fix`
binder is read inside its own body, so its type cannot be synthesized
from the body in one pass, and no binder is annotated.

## spec/fix/typing/from-base

The element type of each `fix` binder is inferred from the `union`
operands of its body that read no binder of unknown type, repeated
until every binder is typed; each body is then checked against `Set`
of its binder's type. A binder whose type cannot be found this way is
a type error.

```adt
fix reach = (from f in follows where f.src == me select f.dst)   -- base operand: Key users
          union (from r in reach, f in follows where f.src == r select f.dst)
in from u in reach select u
```

## spec/fix/typing/annotated

`fix reach: Set (Key users) = ...`. Breaks "no binder needs an
annotation" and adds a form to `fix`.

## spec/fix/typing/join-iterate

Start every binder at no type and join operand types until nothing
changes. Needs a bottom type the type system does not have, and can
widen past what the base cases give.

## spec/fix/typing/least

Each `fix` binder's element type is the least type, under the subtype
order, such that every body checks against `Set` of its binder's type
with the binders at those types. A `fix` with no such types is a type
error.

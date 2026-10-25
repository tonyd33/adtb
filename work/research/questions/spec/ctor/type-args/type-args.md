---
title: "Where a generic constructor's type arguments come from"
pick: one
options:
  expected-then-fields:
    title: "Those fixed by the expected type and the fields' synthesized types; an unfixed one is an error"
  expected-only:
    title: "Only from the expected type"
  written:
    title: "Written explicitly where no expected type fixes them"
---
Raised while implementing the front end against docs/spec/. `Some {
value: 1 }`, `None` and `Err { error: e }` are constructors of generic
prelude sums, and the spec did not say how their type arguments are
found, nor how `p == None` is typed when neither operand has an
expected type.

## spec/ctor/type-args/expected-then-fields

A generic constructor's type arguments are those fixed by the
expected type, when there is one, and by its fields' synthesized
types. One fixed by neither (`None` with no expected type, `Err { error: e }` alone) is
a type error naming the parameter. A comparison operand that is a
constructor, record or literal is checked against the other operand's
type, so `p == None` is well typed.

```adt
from f in fallbacks where f.backup == None select f   -- None : Option (Key identities.Google)
```

## spec/ctor/type-args/expected-only

Type arguments come only from an expected type; `Some { value: 1 }`
alone is refused. Most `let` bindings of a constructor need a
signature.

## spec/ctor/type-args/written

Unfixed arguments are written, `None[Text]`. Adds explicit type
application to the expression grammar.

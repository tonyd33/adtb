---
title: How subtyping passes through type parameters
pick: one
raised-by: [model/user-params]
options:
  derived:
    title: Variance inferred from where a parameter occurs in the declaration
  declared:
    title: Each parameter is annotated covariant, contravariant or invariant
  invariant:
    title: Parameters are invariant
---
`Option (Key identities.Google)` where `Option (Key identities)` is
expected, or `Set Identity.Google` where a `Set Identity`
is. Values are immutable, so a parameter occurring only in fields is
covariant; functions (`lang/abstraction/higher-order`) are
contravariant in arguments.

## types/variance/derived

```
type Option a = None | Some { value: a }       -- a covariant, inferred
from s in sessions select s.identity           -- Option (Key identities.Google) <: Option (Key identities)
```

OCaml infers variance of abstract-free type definitions
(https://ocaml.org/manual/5.2/typedecl.html, unverified). Adding a
field that uses `a` contravariantly is then a schema change that can
break callers; recompile catches it (`evolution/breaks/recompile`).

## types/variance/declared

```
type Option +a = None | Some { value: a }
```

Scala `+A`, OCaml `+'a` in signatures. The declaration states the
contract; a field contradicting it is a declaration error.

## types/variance/invariant

Widening under a parameter needs a `match` that rebuilds the value.
```
match s.identity | None -> None | Some k -> Some { value: k.value }
```

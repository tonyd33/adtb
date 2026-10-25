---
title: References into nested constructor sets need a subtyping rule
pick: any
raised-by: [model/ref-type, model/typed-key]
options:
  ctor-set-subsumption:
    title: ref S is a subtype of ref T whenever S is a subset of T
  explicit-widen:
    title: Widening is an explicit coercion
  live:
    title: Live with each reference type distinct
    excludes: [model/ref-subtyping/ctor-set-subsumption, model/ref-subtyping/explicit-widen]
---
`r.identity : Key identities.EmailPass` passed where `Key identities` is
expected. Narrowing the other way is a runtime test whose answer a later
retag can change.

## model/ref-subtyping/ctor-set-subsumption

Widening is implicit; narrowing is a match on the dereferenced row.

```
match s.identity | EmailPass e -> e.email | Google _ -> "-"
```

Flix restrictable variants type a match over a subset of labels (Madsen
et al., ECOOP 2023). Retag safety is `fk/retag/recheck`.

## model/ref-subtyping/explicit-widen

`widen r.identity` produces a `Key identities`.

## model/ref-subtyping/live

A query unions references of two ctor sets only after dereferencing.

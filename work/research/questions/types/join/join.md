---
title: The type of a match whose arms have different types
pick: one
raised-by: [lang/binder/ctor-row]
options:
  lub:
    title: The least upper bound in the subtype order, records keeping common fields
    links:
      - {rel: prior-art, to: prior/types-mlsub}
  lub-exact-fields:
    title: Least upper bound of constructor sets and keys; record arms must have the same fields
  expected-only:
    title: Arms are checked against an expected type; without one they must agree exactly
    links:
      - {rel: prior-art, to: prior/types-bidir}
---
```
from i in identities select match i
  | Google g -> g          -- Identity.Google
  | Apple a  -> a          -- Identity.Apple
  | EmailPass e -> e
```
The arms meet at `Identity`. Rows of two different sums have no
upper bound: there is no top type. Arms that should stay apart are
tagged as result constructors (`model/sum-results`).

## types/join/lub

`Identity.Google` and `Identity.Apple` join to
`Identity.(Google | Apple)`; `Key S` and `Key T` to `Key (S | T)`;
`{ a: I64, b: Text }` and `{ a: I64 }` to `{ a: I64 }`. MLsub records: the
upper bound has the intersection of the fields.

```
match i | Google g -> { sub: g.sub, team: g.hd } | Apple a -> { sub: a.sub }
-- : { sub: Text }; team silently dropped
```

## types/join/lub-exact-fields

Constructor sets and keys join as in `lub`; anonymous records join
field by field only when their field sets are equal, else a type error
naming the field missing in one arm.

## types/join/expected-only

With a signature, each arm is checked against it (subsumption at the
change of direction). Without one, every arm must synthesize the same
type; widening needs an annotation.

```
fn logins (u: Key users) -> Set Identity = from i in identities ... select match i | Google g -> g | ...
```

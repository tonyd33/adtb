---
title: The inference algorithm for bodies under written signatures
pick: one
raised-by: [lang/inference/signatures]
options:
  bidir-local:
    title: Bidirectional checking with local type-argument synthesis
    links:
      - {rel: prior-art, to: prior/types-bidir}
      - {rel: prior-art, to: prior/types-lti}
  hm-coerce:
    title: Hindley-Milner unification with widening inserted only at coercion sites
  mlsub:
    title: Algebraic subtyping by biunification, then the result checked against the signature
    links:
      - {rel: prior-art, to: prior/types-mlsub}
---
Signatures are written; bodies are inferred. Subtyping exists:
constructor rows below their sum (`lang/binder/ctor-row`), `Key S`
below `Key T` for S a subset of T
(`model/ref-subtyping/ctor-set-subsumption`). Generators over schema
relations fix most binder types, so most of a body synthesizes from
the schema outward.

```
fn oauth_owner (u: Key users) -> Set (Key identities) =
  from i in identities where i.user == u
  select match i | Google g -> g.id | Apple a -> a.id | EmailPass e -> e.id
```

## types/infer/bidir-local

The signature is checked against; generators, field access and
application synthesize; lambdas passed to higher-order functions are
checked against the parameter type; subsumption only where a
synthesized type meets an expected one. Type arguments of generic
calls are solved from local subtyping constraints, picking the least
result type (Pierce and Turner). Errors point at the subterm where
synthesis and expectation disagree. A `let`-bound local function needs
an annotation.

## types/infer/hm-coerce

Unification as in ML; a constructor row or narrower `Key` widens only
where the target type is already known: an argument, a signature
result, a field of a declared type. Rust coercion sites
(https://doc.rust-lang.org/reference/type-coercions.html, unverified).
Local functions need no annotation.

## types/infer/mlsub

Every body gets a principal type with joins and meets; the signature
is checked by the decidable subsumption of type schemes (Dolan and
Mycroft). The most general, and the least local: an error is a failed
constraint whose origin may be far from where it is reported.

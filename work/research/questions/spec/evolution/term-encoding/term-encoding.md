---
title: "How a Term operand of Operator is encoded"
pick: one
options:
  canonical-tokens:
    title: "Text: the resolved term's tokens in canonical spelling, joined by one space, comments dropped"
  ast-rows:
    title: "An abstract syntax tree as catalog values"
---
Raised by the follow-up review of the evolution rulings. The catalog's
`Operator` sum has operands that are terms (fills, `by match` bodies,
predicates), and their encoding was "not yet fixed", so the version
hash had no defined input for them.

## spec/evolution/term-encoding/canonical-tokens

A `Term` operand is `Text`: the token sequence of the resolved term
(every name qualified), each token in a canonical spelling, joined by
one space, comments dropped. `INT` and `DURATION` literals lose leading
zeros; a `DECIMAL` keeps its fractional digits as written, since its
scale is its type; a `FLOAT` loses only leading zeros and the exponent
sign; text literals use a fixed escape set. Values are not normalised
across units: `60s` stays distinct from `1min`. syntax.md defines the
canonical token form once.

```text
term     u.count + 1_000
encoded  u . count + 1000
```

## spec/evolution/term-encoding/ast-rows

Encode the term as an abstract syntax tree of catalog values. Exact,
but a large second schema for the language's syntax, kept in step with
the grammar, to feed one hash.

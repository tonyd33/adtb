---
title: "Where defaults and generators are declared"
pick: one
options:
  relation:
    title: "As relation clauses applied by inserts and retags into that relation"
  type-field:
    title: "On type fields, applied wherever a value is constructed"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/defaults-home/relation

As relation clauses applied by inserts and retags into that relation. Construction stays pure and one type may serve several relation variables.

## spec/defaults-home/type-field

On type fields, applied wherever a value is constructed. Construction would read a clock or a generator.

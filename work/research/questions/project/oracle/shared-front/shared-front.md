---
title: The oracle and the engine share a front end
pick: any
raised-by: [project/oracle/rust-reference, project/oracle/prover-model]
options:
  independent:
    title: The oracle has its own parser and type checker
  typed-input:
    title: Both consume generated typed kernel terms; the front end is tested separately
  live:
    title: Accept that front-end bugs are invisible to differential testing
    excludes: [project/oracle/shared-front/independent, project/oracle/shared-front/typed-input]
---
A parser, desugarer or exhaustiveness bug present in both sides makes
both agree on a wrong answer; PQS notes the same blind spot for
differential testing across engines sharing a bug (arXiv:2001.04174).

## project/oracle/shared-front/independent

Two type checkers must agree on every program; disagreement is itself a
finding. Double the maintenance of every typing rule.

## project/oracle/shared-front/typed-input

The generator emits kernel terms with types; desugaring is checked by
its own property (surface term and its desugaring give equal oracle
results), and typing by soundness properties (well-typed terms never
fail at run time with a type error).

## project/oracle/shared-front/live

Differential testing covers planning and execution only.

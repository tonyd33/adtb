# CQL (categorical query language)

kind: language
home: https://categoricaldata.net

## Model

- Schemas are algebraic profunctors; attributes are observations of a type from an algebraic theory. Instances are presented by generators and relations; generators act like labelled nulls. Schultz, Spivak, Vasilakopoulou, Wisnesky, "Algebraic Databases", TAC 2017, sec. 1.1. [V 2026-10-25] https://arxiv.org/html/1602.03501
- Foreign keys are total functions between entities, constrained by path equations; row ids are terms over generators, not user keys. [U]

## Semantics

- Functor F induces Sigma, Pi, Delta; FQL equivalent to SPCU relational algebra plus key generation. [V 2026-10-25] https://arxiv.org/abs/1212.5303
- Every uber-flower query evaluates to Delta after Pi; Sigma fills mismatched tables with nulls; Delta and Pi preserve type algebras, Sigma does not. [V 2026-10-25] https://arxiv.org/html/1503.03571
- Sigma is union followed by merge. [V 2026-10-25] https://categoricaldata.net/help/Tutorial.html
- No coproduct or sum schema construct found. [V 2026-10-25, absence]

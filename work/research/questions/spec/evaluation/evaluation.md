---
title: "Evaluation order of arguments"
pick: one
options:
  by-value:
    title: "Call by value, left to right within a binding; inlining only of total arguments"
  by-name:
    title: "Substitution; unused arguments never evaluated"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/evaluation/by-value

Call by value, left to right within a binding; inlining only of total arguments. Faults stay predictable and match the surface text.

## spec/evaluation/by-name

Substitution; unused arguments never evaluated. Fault outcomes depend on optimisation.

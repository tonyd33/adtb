---
title: "When a language function is deterministic"
pick: one
options:
  transitive:
    title: "Only when every function it calls, transitively, is deterministic"
  always:
    title: "Always: a function written in the language is deterministic"
---
Raised while writing the migration log rows. The engine set
`deterministic` true for every function written in the language, which
is false for one that calls a nondeterministic host function.

## spec/log-rows/fn-determinism/transitive

A language function is `deterministic` only when every function it
calls, transitively, is deterministic. A cycle of mutually recursive
functions is deterministic when nothing outside the cycle that its
members call is nondeterministic.

## spec/log-rows/fn-determinism/always

Every language function is deterministic. A call to a nondeterministic
host function then reaches keys, indexes and constraints through a
wrapper.

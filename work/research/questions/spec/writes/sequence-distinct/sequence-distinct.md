---
title: "What a sequence key promises against explicit values"
pick: one
options:
  issued-and-held:
    title: "A generated key differs from every value issued and from every key any committed row of the relation has held"
  counter-advance:
    title: "The counter advances past the greatest explicit value at commit"
---
Raised by the implementability review of docs/spec/. writes §2.3 and
model §7.3 stated the promise through a counter that advances, which
is a representation, and left keys held by deleted rows unstated.

## spec/writes/sequence-distinct/issued-and-held

A generated sequence key differs from every value the generator has
issued and from every key value any committed row of the relation has
held. The counter wording moves out of the spec.

## spec/writes/sequence-distinct/counter-advance

The spec states that a counter advances past explicit values. Names a
mechanism, and says nothing about a key held only by a row since
deleted.

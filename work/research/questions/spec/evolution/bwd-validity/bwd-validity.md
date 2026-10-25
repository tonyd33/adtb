---
title: "Whether a backward map yields valid states"
pick: one
options:
  any-state:
    title: "bwd yields states, not necessarily valid ones"
  valid-state:
    title: "bwd yields valid states"
---
Raised by the implementability review of the spec in docs/spec/. The
signature `bwd_op : DB(Σ') → DB(Σ)` claimed a valid state at the older
schema.

## spec/evolution/bwd-validity/any-state

`bwd` yields a state of `Σ`, not necessarily a valid one. After a
constraint of `Σ` is dropped and the drop takes effect, rows violating
it map back unchanged. Readers-first and pending drops keep every live
reader's view valid; the map itself does not.

## spec/evolution/bwd-validity/valid-state

`bwd` yields valid states. False of the drop above, so every law
stated through it would rest on a claim the operators do not meet.

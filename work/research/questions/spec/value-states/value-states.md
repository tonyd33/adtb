---
title: "Whether constructors of value-level sums have write states"
pick: one
options:
  gated:
    title: "`add constructor` on a value-level sum starts declared; `enable T.C` gates storing it in any field; refused while a live bundle pinned before reads a field whose type reaches T, hide new constructors included; storing a declared one is Rejected NotOpen"
  ungated:
    title: "Value-level constructors are writable at once"
---
Raised by the third review of the spec in docs/spec/.

## spec/value-states/gated

`add constructor` on a value-level sum starts declared; `enable T.C` gates storing it in any field; refused while a live bundle pinned before reads a field whose type reaches T, hide new constructors included; storing a declared one is Rejected NotOpen. A pinned reader never decodes a tag its version lacks.

## spec/value-states/ungated

Value-level constructors are writable at once. A writer stores a value an old reader cannot type.

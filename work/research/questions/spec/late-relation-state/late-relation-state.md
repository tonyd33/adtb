---
title: "The write state of a constructor in a relation added after the constructor"
pick: one
options:
  type-state:
    title: "The constructor's state in its type, from catalog::value_states"
  enabled:
    title: "Enabled, as for every constructor of an added relation"
  declared:
    title: "Declared, as after add constructor"
---
Raised by the migration operators. `add constructor T.C` declares `C`
in every relation of `T`; a relation that `add relation r : T` adds
later had no stated state for `C`.

## spec/late-relation-state/type-state

`r.C` starts in `C`'s state in its type: declared after `add
constructor T.C` until `enable T.C`, enabled after it.

## spec/late-relation-state/enabled

Every constructor of a new relation starts enabled, as after `add
type`. A declared constructor would then take writes in a new relation
before readers-first allows it anywhere else.

## spec/late-relation-state/declared

Every constructor added by `add constructor` starts declared in any
relation added later, whatever its state in its type.

---
title: Moving a field up needs it in every constructor
pick: any
raised-by: [evolution/move-field]
options:
  default:
    title: Constructors lacking the field take a declared default
  live:
    title: Live with refusing the move unless every constructor has it
    excludes: [evolution/hoist/default]
---
Moving `created_at` to the sum requires each constructor to carry it at
one type. Moving down is information preserving (unverified). Protobuf
loses data moving fields into or out of a `oneof`.

## evolution/hoist/default

Cambria `add` with a default for constructors that lacked it.

## evolution/hoist/live

Only exact hoists allowed.

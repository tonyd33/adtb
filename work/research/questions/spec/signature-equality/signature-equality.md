---
title: "When two signatures of a family query are equal"
pick: one
options:
  ids:
    title: "Each named type by type id with its constructor and field ids; anonymous types structurally over the same"
  type-ids:
    title: "Named types by type id alone"
---
Raised by the spec-rev30 review (G8). Equality is undecided for an anonymous
result sum, which has no type id, and for a record parameter whose type gains a
field between versions.

## spec/signature-equality/ids

A signature is compared by each named type's type id plus the constructor ids and
field ids it has. An anonymous type is compared structurally over the same. If
anything differs, the bundle is not covered.

## spec/signature-equality/type-ids

Named types compare by type id alone. A type that gains a constructor or field
between versions compares equal, and a client generated for one version receives
values it cannot decode.

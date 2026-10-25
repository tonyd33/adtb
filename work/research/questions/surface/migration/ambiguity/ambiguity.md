---
title: A schema diff cannot tell rename from remove plus add
pick: any
raised-by: [surface/migration/inferred]
options:
  ask:
    title: Ask the developer whenever a removal and an addition could pair
    links:
      - {rel: prior-art, to: prior/surface-gel-migrations}
  stable-ids:
    title: Every constructor and field carries a stable id in the schema
    links:
      - {rel: prior-art, to: prior/surface-protobuf-oneof}
      - {rel: prior-art, to: prior/surface-fdb-record-layer}
  refactor-log:
    title: Renames are recorded beside the schema as they are made
    links:
      - {rel: prior-art, to: prior/surface-ssdt}
  live:
    title: Live with renames inferred as remove plus add
    excludes: [surface/migration/ambiguity/ask, surface/migration/ambiguity/stable-ids, surface/migration/ambiguity/refactor-log]
    links:
      - {rel: prior-art, to: prior/surface-prisma}
---
`Facebook` disappears and `Meta` appears with the same fields. Remove
plus add drops every `Facebook` row; rename keeps them. The same holds
for nest against remove plus add of a new sum, and for move-field.

## surface/migration/ambiguity/ask

Gel prompts with a confidence per step. Non-interactive runs (CI) must
refuse on any ambiguity.

## surface/migration/ambiguity/stable-ids

```
type Provider = Google @1 { gid: Text @1 } | Meta @2 { ... }
```

A rename keeps `@2`, so the diff is exact. Protobuf field numbers,
Cap'n Proto ordinals. The ids leak into every schema file.

## surface/migration/ambiguity/refactor-log

`adt rename Identity.Facebook Meta` edits the schema file and appends
to a log that the diff consumes. SSDT `.refactorlog`; losing the log
turns past renames back into drops.

## surface/migration/ambiguity/live

Prisma: the generated file drops and adds; the developer edits it.

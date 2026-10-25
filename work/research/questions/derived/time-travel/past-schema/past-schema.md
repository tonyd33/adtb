---
title: A past state was written under an earlier schema
pick: one
raised-by: [derived/time-travel/window, derived/time-travel/versioned, derived/time-travel/everything]
options:
  as-written:
    title: A past read is typed against the schema version current at that point
    links:
      - {rel: gains, to: surface/version/counter}
  translated:
    title: Past states are read through the operator log under today's schema
    links:
      - {rel: gains, to: evolution/merge}
---
The schema is the fold of a linear operator log (`surface/source/op-log`,
`surface/version/counter`). Constructor `Pending` may not exist at
version 4120.

## derived/time-travel/past-schema/as-written

`at version 4120 { ... }` is checked against schema version of 4120;
a registered query reading the past names that version, as bundles pin
theirs.

## derived/time-travel/past-schema/translated

Operators carry inverses or declared losses (`evolution/smo`), so a
past state maps forward; a lossy operator in between makes some past
reads refused.

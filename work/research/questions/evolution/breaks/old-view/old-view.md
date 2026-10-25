---
title: Old schema versions meet rows of new constructors
pick: one
raised-by: [evolution/breaks/versioned]
options:
  hide:
    title: Hide them from the old version
  unknown:
    title: Show them as an unknown constructor
    links:
      - {rel: same-as, to: query/unknown-arm}
  refuse:
    title: Refuse old-version reads that would range over them
---
An `Apple` row read through the two-constructor version. Hiding skews
old aggregates silently, as BiDEL's auxiliary tables do for rows
outside a `SPLIT`.

## evolution/breaks/old-view/hide

The old version's sum is filtered to its constructors.

## evolution/breaks/old-view/unknown

The old version gains an opaque constructor.

## evolution/breaks/old-view/refuse

Queries over the whole sum fail in the old version once new rows exist.

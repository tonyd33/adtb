---
title: Index tags shift when constructors change
pick: any
raised-by: [surface/sum-encoding/index-tag]
options:
  descriptor:
    title: Each result names its descriptor; a mismatched client gets the new one
    links:
      - {rel: prior-art, to: prior/surface-gel-migrations}
  per-version:
    title: Tags index the constructor list of the bundle's pinned version
  live:
    title: Live with clients misreading tags after a change
    excludes: [surface/sum-encoding/shift/descriptor, surface/sum-encoding/shift/per-version]
---
A client compiled when `Identity = EmailPass | Facebook | Google`
reads tag 1 as `Facebook` after `Facebook` is removed.

## surface/sum-encoding/shift/descriptor

Gel's output descriptor ids: the client declares the id it was built
for, and a mismatch is reported before any data.

## surface/sum-encoding/shift/per-version

The engine encodes each result against the constructor list of the
version the calling bundle names, translating through the operators.

## surface/sum-encoding/shift/live

Silent misdecoding.

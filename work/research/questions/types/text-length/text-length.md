---
title: What text length and slicing count
pick: one
raised-by: [model/text]
options:
  scalars:
    title: Unicode scalar values
  graphemes:
    title: Extended grapheme clusters
  bytes:
    title: UTF-8 bytes
  explicit:
    title: No length; byte, scalar and grapheme counts are separate functions
---
`length(name) <= 50` in a domain check, `substring(t, 0, 10)`.

## types/text-length/scalars

Postgres `length` in UTF-8, Python `len`.

## types/text-length/graphemes

Swift `count`. Segmentation (UAX #29) changes between Unicode
versions, so a stored check can change verdict on upgrade.

## types/text-length/bytes

Rust `len`; slicing at a non-boundary faults.

## types/text-length/explicit

`bytes_len(t)`, `scalar_len(t)`, `grapheme_len(t)`.

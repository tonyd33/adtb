---
title: "The textual form of a SortKey"
pick: one
raised-by: [spec/textual-forms/interop]
options:
  hex:
    title: "0x hex, as Bytes"
  none:
    title: "No textual form; a SortKey is never printed"
---
Raised by the reference interpreter's printer. interop.md §1.1 promises
every value of every type a printed form, and §1.3 gave none for
`SortKey`.

## spec/textual-forms/sortkey/hex

A `SortKey` prints and reads as `Bytes` do, `0x` then two hex digits
per byte: `0x2b4f01`. Its order is the order of its bytes, so the
text of two keys of one length sorts as the keys do.

## spec/textual-forms/sortkey/none

A `SortKey` has no form, and a query or dump that would print one is
refused. Breaks the promise of §1.1 and makes a stored `SortKey` field
undumpable.

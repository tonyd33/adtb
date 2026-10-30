---
title: "What the admission byte bound counts"
pick: one
options:
  held:
    title: "Arguments, ad hoc text and parameters, and a fixed per-call overhead, as admitted; Append batches after admission are not counted"
  encoded:
    title: "The encoded size of the request on the wire"
---
Raised by the spec-rev30 review (G17).

## spec/admission-bytes/held

The bytes a call holds are the memory of its arguments, of the text and parameters
of ad hoc text, and of a fixed overhead per call, as the call is admitted. The
batches of an Append that arrive after admission are not counted.

## spec/admission-bytes/encoded

Count the request's wire size, which an in-process call does not have.

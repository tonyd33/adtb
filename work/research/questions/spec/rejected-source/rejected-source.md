---
title: "What RejectedRow.source holds for a source relation"
pick: one
options:
  unqualified:
    title: "The relation's unqualified name"
  qualified:
    title: "Its qualified name, m::r"
---
Raised by the reference interpreter's writes. interop.md §4.5 said
`source` is "the name of the relation the rows were read from".

## spec/rejected-source/unqualified

`source` is the relation's name as declared, `identity_stage`, with no
module. A file source is its path.

## spec/rejected-source/qualified

`source` is `auth::identity_stage`. Unambiguous across modules, but a
temp relation has no module, and a rejects relation normally sits
beside the one load it serves.

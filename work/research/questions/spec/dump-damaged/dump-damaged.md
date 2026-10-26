---
title: "Which error a restore of a damaged dump fails with"
pick: one
options:
  offset:
    title: "DumpDamaged { offset: I64 }, the byte offset of the first part that fails to authenticate or decode"
  bad-input:
    title: "The generic BadInput reason"
---
Raised by `spec-dump-damaged-error`. A restore with the wrong passphrase
fails as a restore of a damaged dump does, and no error names that
failure.

## spec/dump-damaged/offset

A restore that cannot authenticate or decode a dump fails with
`DumpDamaged { offset: I64 }` of the restore error sum, `offset` the
byte offset of the first part that fails. A wrong passphrase fails the
first sealed part.

## spec/dump-damaged/bad-input

Reuse `BadInput` with the dump's path. Carries a line, which a binary
dump does not have.

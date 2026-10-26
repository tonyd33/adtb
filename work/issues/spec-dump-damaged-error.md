---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 296
summary: "the spec says a wrong passphrase fails as a damaged dump does, but names no error for a damaged dump"
closed: 2026-10-25
tags:
  - spec
---

# Bug: No error for a damaged dump

## Report

`docs/spec/interop.md` §6.1 says a restore with the wrong passphrase
fails as a restore of a damaged dump does. No error constructor names
that failure.

My ruling: a restore that cannot authenticate or decode a dump fails
with `DumpDamaged { offset: I64 }`, the byte offset of the first part
that fails. It sits in the restore error sum of `docs/spec/catalog.md`
§4, and §6.1 names it.

## 2026-10-25: resolved

In the spec with `docs: record the spec4 rulings and make every spec example check`. Point `spec4`, for Tony to adopt after `spec2`; its temp names ruling supersedes `spec2`'s.

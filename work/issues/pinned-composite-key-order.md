---
created-at: 2026-11-03
status: open
kind: bug
priority: 125
summary: "a rename that changes a composite key's field sort order is not permuted for a pinned bundle, so its inserted keys and key equality read the fields swapped"
---

# Pinned composite key field order

## Report

A composite key's value is laid out with its fields sorted by name. Found reviewing `fix: name every Prim in the translation analyses`: with `add type Pair = { a: Text, b: Text }`, `add relation pairs : Pair key (a, b)` and a bundle pinned before `rename field Pair.a to x`, a pinned `txn` inserting `{a: "1", b: "2"}` and returning `key p` yields `Record([Text("2"), Text("1")])` under the pinned form `(a, b)`, so the client reads a = "2", b = "1"; and `from p in pairs where key p == pairs.key(a: "1", b: "2") select p.b` answers `[]` where the pin answers `["2"]`. Wrong answers in normal use. Defect class: identity by position in the key record. Direction: translation maps a key value field by field identity, never by sorted position; a search over every rename of a composite key field that reorders the sort, for inserts, key equality, references and print.

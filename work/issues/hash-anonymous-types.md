---
created-at: 2026-10-25
status: open
kind: feature
priority: 510
summary: "hashing of anonymous records and sums for grouping is undefined"
tags: [design]
---

# Feature: hashing of anonymous records and sums

## Report

`docs/design/indexes.md` §5 says the hash of a value is a hash of its
canonical key encoding, consistent with `==` for every type, sums and
records included. Grouping by an anonymous record or sum needs that
hash, and the design does not define the encoding, and so the hash, of
an anonymous record or sum.

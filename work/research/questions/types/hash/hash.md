---
title: How values hash
pick: one
raised-by: [btree/hash/eq-unique]
options:
  key-bytes:
    title: The hash of a value is a hash of its canonical key encoding
    needs: [btree/keyenc/canon/canonicalize]
  per-type:
    title: Each type supplies a hash function consistent with its ==
---
Hash joins, hash aggregation and partitioning need `a == b` to imply
`hash(a) == hash(b)`.

## types/hash/key-bytes

Since equal values encode to equal bytes
(`btree/keyenc/canon/canonicalize`), hashing the memcmp encoding is
consistent with `==` by construction, including for sums and records.
Persisted hashes (partitioning) use a fixed, versioned function;
in-memory tables may seed it per process against collision attacks.

## types/hash/per-type

Postgres hash opclass support functions, one per type, each kept
consistent with its btree equality by hand
(https://www.postgresql.org/docs/current/xindex.html, unverified).

---
title: An opaque type must meet the key-encoding rules to be indexed
pick: any
raised-by: [interop/scalar/opaque]
options:
  ext-encoder:
    title: The extension supplies an order-preserving canonical encoder, property-tested at install
  no-key:
    title: Opaque types cannot be keys or B-tree indexed; only extension index kinds take them
  live:
    title: Live with indexes ordered by an encoder nobody checked
    excludes: [interop/scalar/keyenc/ext-encoder, interop/scalar/keyenc/no-key]
---
Every key type has an order-preserving byte encoding
(`btree/keyenc/memcmp`) in which equal values encode equally
(`btree/keyenc/canon/canonicalize`).

## interop/scalar/keyenc/ext-encoder

`key_encode(a) < key_encode(b)` iff `compare(a, b) < 0`, checked on
generated values at install.

## interop/scalar/keyenc/no-key

A `Geometry` field is indexed only by a spatial index kind
(`interop/index-am`).

## interop/scalar/keyenc/live

Range scans miss rows.

---
created-at: 2026-10-25
status: open
kind: feature
priority: 502
summary: "no order-preserving key encoding, ovarint constructor ids or union index entries"
depends-on:
  - hash-anonymous-types
tags:
  - engine
---

# Feature: order-preserving key encoding

## Report

Implement `docs/design/indexes.md` §2 to §5: the order-preserving
encoding of every key type, `ovarint` constructor ids, union index
entries `(key, ctor id, tid)`, order keys, and the hash of a value as
the hash of its canonical encoding. Property tests: encoding order
equals value order, and equal values hash equal.

Prerequisite: `hash-anonymous-types.md`, the encoding of anonymous
records and sums.

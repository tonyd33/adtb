---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 502
summary: "no order-preserving key encoding, ovarint constructor ids or union index entries"
closed: 2026-10-25
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

## 2026-10-25: mostly done

`adtb::key`, landed in `feat(engine): add order-preserving index key
encoding`, already has §2 to §5 for scalars, records, declared sums,
`ovarint`, entries and hashing. What remains is the anonymous sum
encoding ruled in §3, and the order key allocation of §4 if `adtb::key`
does not have it.

## 2026-10-25: resolved

Landed in `feat(btree): add B-link tree indexes, order keys and anonymous key types`: anonymous record and sum key types, and order key allocation (`OrderKey::between`, `OrderKey::spread`).

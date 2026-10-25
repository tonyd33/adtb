---
title: Which specializations the schema admits
pick: one
options:
  disjoint-total:
    title: Only disjoint and total, i.e. sum types
  disjoint-partial:
    title: Disjoint, totality optional per sum
  eer:
    title: Full EER specialization, overlap and partiality allowed
    links:
      - {rel: prior-art, to: prior/eer}
      - {rel: prior-art, to: prior/third-manifesto}
      - {rel: prior-art, to: prior/datomic}
---
EER specialization is annotated disjoint or overlapping and total or
partial. A sum type is the disjoint and total case. Admitting the others
makes "every identity has exactly one variant" a declaration rather than
the meaning of the type.

## sum/disjoint-total

Every sum is exclusive and total by its type. No annotation. Overlap
lives in relationships: an `identities` row with `user: Key users` lets one user hold both a
password and a Google login while each identity is one constructor.

## sum/disjoint-partial

A sum may be declared partial: a row of the parent need not belong to
any constructor. Equivalent to a sum with an implicit empty constructor;
protobuf `oneof` is at most one.

## sum/eer

A specialization declares `disjoint | overlapping` and `total | partial`.
A partial specialization behaves like a sum with an implicit empty
constructor; an overlapping one is a set of optional facets, not a sum.

---
title: "What a family call revoke without a signature matches, and per whom a family row is held"
pick: one
options:
  widen:
    title: "It matches the family row whatever its signature; one row per grantee, family, query and grantor"
  exact:
    title: "It must name the recorded signature"
---
Raised by the spec-rev30 review round 2 (N1).

## spec/family-revoke/widen

A family call revoke written without a signature matches the family row whatever
its signature, a third widening beside spec/revoke-widening/keep. There is one
family row per grantee, family, query and grantor, in line with
spec/revoke-grantor/per-grantor.

## spec/family-revoke/exact

Every family revoke spells `at v` and the signature.

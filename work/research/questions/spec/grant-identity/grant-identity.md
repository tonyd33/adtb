---
title: "How a revoke names a grant after a carry"
pick: one
options:
  target-text:
    title: "A grant's target text is its identity; grant and revoke accept that text, and a revoke matches it exactly"
  resolved:
    title: "A revoke is resolved like a grant and matches by subset of the held field set"
---
Raised by the spec-rev30 review (G1, G4). After a split, a carried grant can
hold a shape no 3.3-valid statement writes, such as
`identities.(Google | Apple) frozen (id, user, Apple.sub)`. Under exact matching
of resolved statements it cannot be revoked, and removing only its option by
revoke then grant changes more than the option.

## spec/grant-identity/target-text

The target text of a row of `catalog::grants` is the grant's identity. Grant and
revoke accept it as written, `frozen (...)` lists and `C.f` field names included,
and a revoke matches a row exactly by its current target text. A carried grant can
always be revoked, and removing an option is revoke then grant with the same text;
spec/option-revoke/replace stands. security.md 3.2 keeps grant-carry rulings 6
and 12 as two widenings of exact matching (a revoke without a field list matches
every frozen set; a retag revoke without `except` matches whatever pairs are
excepted); that combination is the spec writer's reading, not part of the ruling.

## spec/grant-identity/resolved

A revoke resolves its statement as a grant does and matches a held grant whose
field set contains the revoke's. A carried shape is still not writable as a
grant, so revoke then grant cannot restore it.

---
title: "Whether exact target matching of a revoke keeps the two widenings"
pick: one
options:
  keep:
    title: "A revoke without a field list matches every frozen field set; a retag revoke without except matches whatever pairs are excepted"
  exact:
    title: "No widenings: a revoke matches only the exact target text"
---
Raised by the spec-rev30 review round 2. spec/grant-identity/target-text makes a
revoke match a row exactly by its current target text, while grant-carry rulings 6
and 12 let an unlisted revoke remove every frozen set and a retag revoke remove a
retag whatever pairs it excepts.

## spec/revoke-widening/keep

Both widenings hold on top of exact matching, so `revoke read r.C from x` and
`revoke retag a -> b from x` keep removing what the plain grant gave.

## spec/revoke-widening/exact

Every revoke must spell `frozen (...)` or `except (...)` as the row prints it.

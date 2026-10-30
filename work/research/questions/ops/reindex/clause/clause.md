---
title: "What `reindex r index (f)` names when it has no `in S`, and whether `reindex r` includes the key index"
pick: one
raised-by: [ops/reindex/online]
options:
  in-disambiguates:
    title: "`in S` is optional and only disambiguates; `reindex r` rebuilds every index including the key index"
  in-required:
    title: "`in S` is always written; the key index has its own form"
---
Raised by the milestone 9 audit. Catalog §11.4 always shows `in S`, and
`reindex identities index (user)` fails to parse, so an index declared
with no scope cannot be named. A key index can be quarantined, and
`reindex r` is the only form that could reach it.

## ops/reindex/clause/in-disambiguates

`reindex r index (f)` names the one index of `r` declared with those
fields. When several are declared over different constructor sets, it
is invalid and `in S` selects one. "Every index" in §11.4 means every
index the engine keeps for `r`, the key index and uniques included, so a
quarantined key index has a way back with no new syntax.

## ops/reindex/clause/in-required

An index declared with no scope would need a spelled-out constructor set
it was never declared with, and the key index would need new syntax for
a case `reindex r` already covers.

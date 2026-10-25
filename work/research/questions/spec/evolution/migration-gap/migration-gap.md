---
title: "What a gap in a migration set does"
pick: one
options:
  whole-set:
    title: "The whole set is refused before any migration applies"
  prefix:
    title: "Migrations before the gap apply; the rest are refused"
---
Raised by the implementability review of the spec in docs/spec/.

## spec/evolution/migration-gap/whole-set

A set with a gap in its numbering is refused whole, before any
migration of it applies: `apply migrations "0011" "0013"` at version
10 applies nothing.

## spec/evolution/migration-gap/prefix

`0011` applies and `0013` is refused. A deploy whose set is wrong is
left half applied, at a version nobody tested.

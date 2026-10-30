---
title: "How a family grant's target text fixes the ids of its signature"
pick: one
options:
  at-version:
    title: "The text names the log version at which its type names resolve; equality is by the ids those names had then"
  ids:
    title: "The text prints type and constructor ids"
---
Raised by the spec-rev30 review round 2 (N2). Dropping and re-adding `type Token`
leaves the names equal and the ids different.

## spec/family-signature-version/at-version

The target is `FAMILY::q at v (params) -> type`, `v` the highest version a live
bundle declaring the query pins; equality uses the ids the names had at `v`. The
form parses under the grant grammar.

## spec/family-signature-version/ids

Ids in the text are unreadable and still need a version for named types.

---
title: A structural operator has no image for some constraint
pick: any
raised-by: [constraint/evolve/carry]
options:
  refuse:
    title: The operator is refused until the constraint is rewritten or dropped
  declared-loss:
    title: The constraint is dropped and the operator flagged lossy
  live:
    title: Live with only the constraints the rewrite rules cover
    excludes: [constraint/evolve/unmapped/refuse, constraint/evolve/unmapped/declared-loss]
---
`remove constructor Identity.Facebook` with `constraint some_facebook
= ...` requiring a Facebook row; a merge without a discriminator, where
`unique (gid) in Google` and `unique (sub) in Apple` have no joint form; a split whose
routing function a general assertion's query does not see.

## constraint/evolve/unmapped/refuse

The schema change reports each constraint without an image; the user
drops it with an ICMO first, making the loss explicit there.

## constraint/evolve/unmapped/declared-loss

As PRISM flags `MERGE TABLE` not information preserving.

## constraint/evolve/unmapped/live

Constraints outside the covered shapes are recompiled against the new
schema like any registered query and fail or pass as they fall.

---
title: Plans and compiled code stored at registration belong to one engine release
pick: any
raised-by: [exec/plan-cache/at-register]
options:
  stamp-replan:
    title: Each stored plan carries the engine release; a new binary replans on open
  lazy-replan:
    title: A plan from an older release is replanned on its first call
  live:
    title: Live with it
    excludes: [ops/plan-release/stamp-replan, ops/plan-release/lazy-replan]
---
A stored plan names physical operators and, under `exec/model/adaptive`,
may hold generated code. A new binary may not have the old operators, or
may plan better. Reading old file formats (settled `read-old`) does not
make old plans runnable.

## ops/plan-release/stamp-replan

SQL Server stores `engine_version` with each Query Store plan. Opening
a file with plans from another release replans every registered query
before serving, against the same schema and statistics; a pinned plan
that the new planner cannot reproduce is reported. Open time grows
with the number of registered queries.

## ops/plan-release/lazy-replan

Open is fast; first calls after upgrade pay planning, and plan
regressions from the new planner surface under load.

## ops/plan-release/live

Plans in an old release's form are run by every later release.

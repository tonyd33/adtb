---
title: Two branches each add a migration onto one parent
pick: any
raised-by: [evolution/smo]
options:
  linear:
    title: History is a chain; the later branch rebases its operators
  commute:
    title: Migrations whose operators commute merge without a rebase
  live:
    title: Live with merge conflicts in migration files
    excludes: [surface/version/history/linear, surface/version/history/commute]
---
Parallel development adds `Apple` on one branch and renames `Facebook`
on another, both onto 0007.

## surface/version/history/linear

The second branch's migration is regenerated onto the first's. Rails
timestamps order by time and ignore the conflict.

## surface/version/history/commute

Typed operators touching disjoint constructors commute, so the engine
can order them and prove the result equal. Operators touching the same
constructor need a rebase.

## surface/version/history/live

The developer resolves the conflict in the files.

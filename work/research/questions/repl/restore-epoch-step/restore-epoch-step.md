---
title: How two restores of one backup get different epochs
pick: one
options:
  random-step:
    title: A restore raises the epoch by a random step
  history-id:
    title: Positions carry a minted history id beside the epoch
  counter:
    title: The epoch rises by one, and two restores share positions
---
Restoring a backup twice, each restored file then taking different commits. A position from one must not read as on the other.

## repl/restore-epoch-step/random-step

The epoch stays one increasing I64 and `Position` keeps its shape, so nothing new is exposed. A step up to 2^32 makes a collision negligible and leaves room for 2^31 restores. The epoch rises at promotions and restores rather than counting them.

## repl/restore-epoch-step/history-id

Certain rather than probable, at the cost of a new field in a public prelude type for a case the epoch can cover.

## repl/restore-epoch-step/counter

Contradicts repl/restore-epoch/bump, whose point is that two histories do not share positions.

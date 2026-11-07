---
created-at: 2026-11-06
status: resolved
kind: bug
priority: 150
summary: a_crashed_tree_recovers_a_prefix_of_its_operations takes about 22 s alone and times out under full-suite load
closed: 2026-11-06
tags:
  - class-none
---

# Bug: the btree crash-prefix test times out under load

## Report

`btree::a_crashed_tree_recovers_a_prefix_of_its_operations` takes about 22 s alone, both on main and on the damage-log change 2 tree (2026-11-06), and was terminated at 60 s in a full `check --changed` suite. A full check fails on it without any change to the btree.

Expected: the test fits its slow-timeout with margin under the full suite, by a smaller search, a bound on the model's work, or a declared slower group with the reason stated.

## 2026-11-06: raised to 150

It timed out in three of today's full checks (damage-log change 2, migration identity change 2, the sim split), each time failing a check on a change that does not touch the btree, and in a baseline suite under load; alone it takes 22 to 32 s against the 60 s limit.

## 2026-11-06: fixed

`test: seek an incomplete split only in the first cut of the btree crash-prefix test`. About 22 of its 24 s went to `incomplete_at`, which tried every log force inside an insert in every cut until a split was repaired; only one cut in 6 seeds ever found one. Now the first two runs insert entries sharing a 300-byte prefix through the 5-frame pool (the shape `record_splits` uses), only the first cut seeks a force that leaves a split incomplete, earliest first, and the prefix property is still checked at every cut against the model replayed from the operations. Reach is asserted: incomplete splits repaired at least half a split per seed (every one of 48 deep seeds reached at least one, 54 in all), and a recovered tree three levels high (all 48). Alone 2.2 s default, about 23 s deep (48 seeds). Review: no blocker; the first version's floor was reached by about 4% of seeds, fixed as above.

## 2026-11-06: resolved

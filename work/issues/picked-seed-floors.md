---
created-at: 2026-11-02
status: open
kind: bug
priority: 360
summary: pager search floors on picked seeds break whenever an unrelated change shifts the op stream, and the deep floor on pages past their log rests on one seed
tags:
  - quality
  - test
---

# Picked seed floors

## Report

The test-audit-a floors in crates/adtb-sim/tests/pager.rs run quick and default on picked seeds, chosen because they reach each finding kind. The create-aside track changed only the ops `Pager::create` issues, and three picked starts stopped reaching their kind (`threads_sharing_a_pager_under_every_fault` log findings, `every_fault_at_once` and `misdirected_and_lost_writes_are_reported_never_returned` page findings in quick) while deep rates held: on threads, 6 of 64 seeds reach a log finding on both trees, a disjoint set. The track re-picked them; it was abandoned, so main keeps the old seeds. The next change to create, open or the workload will break them again.

The deep-only floor "pages past their log" in the misdirected test (1 per 96) rested on seed 337 alone and reached 0 on the create-aside tree. `Unlogged` stays covered by the deterministic tests in pager.rs and verify.rs.

Direction: a floor in quick and default must hold over the base rate, not a picked start: either raise the seed count to where the kind's measured rate makes a miss rare, or reach the kind by construction (a fault placed on the op that yields it) and keep the random sweep for no-wrong-answer only. The deep `Unlogged` floor needs a workload that reaches a page past its log on more than one seed, or goes.

From the create-aside delta review: on that tree, of eight 8-seed windows across 800..864 for `threads_sharing_a_pager_under_every_fault`, three passed; three failed on zero log findings and one on reopens. The floor is marginal for an 8-seed window regardless of the start.

## 2026-11-03: from create-aside-2

Measured over 800..864 on main and on the create-aside-2 tree, for `threads_sharing_a_pager_under_every_fault`: refused opens about 7% of crashes on both; at the old bound of one per 12 crashes, 46 of 57 eight-seed starts held on that tree and 56 on main, at one per 8, 56 on main. A log finding is reached on about 1 seed in 10 on both, so no 8-seed window holds a floor of one. Ruled in that track: the bound becomes one per 6 crashes (all 57 starts hold on both), and the log-findings floor runs under deep only. The other tests' picked floors are unchanged.

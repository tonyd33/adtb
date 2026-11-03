---
created-at: 2026-11-02
status: resolved
closed: 2026-11-03
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

## 2026-11-03: resolved

`test: reach each pager finding kind by construction` (crates/adtb-sim/tests/pager.rs only). Three deterministic tests reach their kind on every seed: `a_misdirected_page_write_is_refused_where_it_was_meant_and_where_it_landed`, `a_lost_page_write_is_reported_and_refused_on_reopen`, `a_lost_log_write_refuses_the_reopen_at_its_gap`; mutants of the read-path stale filter, the pid in the page checksum, verify's Stale arm and the recovery Gap check each fail one by name in quick, default and deep. The sweeps keep their wrong-answer and lost-row judging in every profile; their kind floors run under deep only, as named constants stating the rule from the measured rate. `picked` is gone. The threads reopen floor is 13, half the measured mean over 8 seeds, and holds over every 8-seed window of 0..256. Lead ruling, changing the create-aside-2 one: `CRASHES_PER_REFUSAL` is 3, since one per 6 failed 6 of those 249 windows (mean 7.2% of crashes, worst window 25%). The deep "pages past their log" floor is removed, reached on 0 of 64 seeds; `Unlogged` stays covered by three pager.rs tests and verify.rs `a_page_past_the_log_end_is_an_unlogged_finding_of_its_relation`. Full check 3650 passed. Found on the way: bitmap-unreadable-no-finding, which fails deep `bit_rot_and_latent_errors_are_reported_never_returned` at seed 248.

Left: in default, only the constructed tests catch a pager that stops refusing Stale reads or stops treating a log gap as fatal; the random sweeps do not. Under deep the threads reopen floor is not scaled past 8 seeds (`share` returns 13 over 64).

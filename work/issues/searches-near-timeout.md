---
created-at: 2026-11-16
status: open
kind: bug
priority: 190
summary: a_pinned_insert_translates_across_every_generated_history_of_two_relations and each_binder_with_each_access_of_tone_is_stale_after_enable take about 30 s alone against a 60 s limit and time out when two tracks share the machine
tags:
  - test
  - class-unbounded-work
---

# Two more searches run near their timeout

Main's full check at 6197a97f on 2026-11-16 timed out four tests at 60 s at load average 31, while two tracks built and tested. Two are filed (laws-revert-test-near-timeout, conflict-branch-test-near-timeout); the other two are:

- `adtb-lang translate::tests::a_pinned_insert_translates_across_every_generated_history_of_two_relations`: 27.8 s alone
- `adtb-sim::server bundles::each_binder_with_each_access_of_tone_is_stale_after_enable`: 33.5 s alone

(alone: the four run together through `work/tools/suite` on main.)

Fix direction: as laws-revert-test-near-timeout: measure, then in the default profile drop a case only when a kept case shares its outcome and its code path, the deep profile keeping all; record each class where its outcome is observed and assert every class was reached; a mutant that breaks one path fails the default run.

Defect class: Unbounded work over size, depth or sharing.

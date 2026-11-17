---
created-at: 2026-11-16
status: open
kind: bug
priority: 180
summary: a_conflict_branch_naming_old_reads_the_relation_unless_new_constructors_are_hidden takes 27 s alone against a 60 s limit and times out when two tracks share the machine
tags:
  - test
  - class-unbounded-work
---

# The conflict branch bundle test runs near its timeout

`adtb-sim::server bundles::a_conflict_branch_naming_old_reads_the_relation_unless_new_constructors_are_hidden` timed out at 60 s twice on 2026-11-16 while two tracks ran at once: in the structure-test-modules track's `check --changed` and in main's full check at 8be12b90. Run alone through `work/tools/suite` it passes in 27 s, flagged slow. Neither change touched the engine.

Every full check run while another track builds or tests can fail on it, and a two-track limit is now the norm.

Fix direction: find what the test spends 27 s on (it is the slowest of its binary) and cut the work to what its property needs, or give it the slots it uses in .config/nextest.toml; the test keeps asserting the same outcome.

Defect class: Unbounded work over size, depth or sharing.

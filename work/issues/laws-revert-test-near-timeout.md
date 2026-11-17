---
created-at: 2026-11-16
status: open
kind: bug
priority: 185
summary: "every_invertible_operator_and_its_revert_restore_the_source_before_it takes 30 to 60 s against a 60 s limit and timed out in main's check under two-track load"
tags:
  - test
  - class-unbounded-work
---

# The invertible operator law search runs near its timeout

`adtb-lang evolve::tests::laws::every_invertible_operator_and_its_revert_restore_the_source_before_it` timed out at 60 s in main's full check at 2a20085f on 2026-11-16, while another track built and tested. It passed at 59.9 s in that track's own check. Alone through `work/tools/suite` on main it takes 29.5 s; on the tree before 2a20085f it took 42 s run unlocked while another track built, so the change did not slow it.

Same cause as conflict-branch-test-near-timeout: a search whose default profile does work near its slot's timeout fails whenever two tracks share the machine.

Fix direction: measure where the time goes; in the default profile keep one case per distinct operator and engine path (drop only cases whose outcome and path a kept case shares), the deep profile keeping all; each phase asserts what it reached where the outcome is recorded.

Defect class: Unbounded work over size, depth or sharing.

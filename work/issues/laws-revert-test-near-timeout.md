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

## 2026-11-17, measured; thinning parked

Measured alone through `suite` (receipt-laws-test-r1, local): default 26 to 30 s; per case about 70 ms, spread over the structural operators (remove constructor, merge, split, nest, unnest). Time: apply 29%, `Source::resolved` 25% (about one apply, as for a client), the test's `outside` helper replaying the migration a second time 22%, `differences` 20%. No step dominates. A thinning that skips a hand-kept list of shape-blind operators after the first shape took it to 22 s; not landed, since it saves 15% at the cost of a list that can drift (parked/laws-test-thin, local, not pushed). The levers that remain: share one replay between `resolved` and `outside` (needs evolve/), and memoise shared setup migrations. The lead runs main's full check with at most one track building, which keeps this test under its limit meanwhile.

## 2026-11-17: timed out again

Timed out at 60 s twice under load beside translate::tests::a_pinned_insert_translates_across_every_generated_history_of_two_relations, both passing alone (21.7 s and 25.4 s). Measured alone on main: 54.5 s and 44.1 s. The machine-lock removal makes the 60 s kill a hang guard.

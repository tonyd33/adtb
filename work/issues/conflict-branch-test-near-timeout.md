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

## 2026-11-16, default profile thinned; still open

Landed as sruskmvk 6197a97f: under the plain header every row runs; the hidden header keeps one row per added constructor per phase (same outcome for every hidden row); the coverage assertion records classes where outcomes are observed; a `names_row` mutant fails the default run. Default 22 to 25 s alone (was 27 to 32), deep 156 outcomes. It still timed out in main's check at load 31, so the ticket stays open: the remaining cost is about 87 database opens at 70 to 250 ms each.

## 2026-11-16, quality review: the thinned hidden live phase reaches only a free case

In the default profile the hidden header's live phase takes the first live-eligible row per link kind (crates/adtb-sim/tests/server/bundles.rs:843). For `url` that is row 1, `skip`, which is `free`; no `body` row is live-eligible (`branch_case`, bundles.rs:750-756, rows 19 and 28 bind `written`). So no default run checks while live that hiding new constructors lifts the `enable` refusal of a `uses` branch: the `blocks && hides` live branch is reached only in the deep profile. `outcome_class` (bundles.rs:762) reduces a hidden case to its link kind and drops `uses`/`free`, so the coverage assertion passes. The doc's "every case ends the same" holds for the call's result, not for whether enable is refused. Fix direction: pick each link kind's hidden representative among the `uses` rows, or keep `uses` in the hidden outcome class.

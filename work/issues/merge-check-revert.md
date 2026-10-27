---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "reverting a merge that carried a one-constructor check is refused with \"no `provider` in scope\""
tags:
  - lang
---

# Bug: a merge's carried check breaks its revert

## Report

A merge carries a check over one of the merged constructors as a `match` on the discriminator. Its revert drops the discriminator field but leaves the carried `match`, so the revert is refused with "no `provider` in scope". The revert must restore the check on the constructor it came from. The ignored tests `a_revert_of_a_merge_restores_the_check_it_carried` in `crates/adtb-lang/src/evolve_tests.rs` and `merges_carrying_checks_keep_their_laws` in `crates/adtb-testkit/tests/differential.rs` (seed 187, "no `fx3` in scope" at migration 4) state it. Start from the merge revert path in `evolve/revert.rs` and `merged_checks` in `evolve/ctor.rs`.

---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "reverting a merge that carried a one-constructor check is refused with \"no `provider` in scope\""
closed: 2026-10-27
tags:
  - lang
---

# Bug: a merge's carried check breaks its revert

## Report

A merge carries a check over one of the merged constructors as a `match` on the discriminator. Its revert drops the discriminator field but leaves the carried `match`, so the revert is refused with "no `provider` in scope". The revert must restore the check on the constructor it came from. The ignored tests `a_revert_of_a_merge_restores_the_check_it_carried` in `crates/adtb-lang/src/evolve_tests.rs` and `merges_carrying_checks_keep_their_laws` in `crates/adtb-testkit/tests/differential.rs` (seed 187, "no `fx3` in scope" at migration 4) state it. Start from the merge revert path in `evolve/revert.rs` and `merged_checks` in `evolve/ctor.rs`.

## 2026-10-27: done

The inverse of a merge sets each check over the merged constructors back to its text before the merge in the same step as the split, so no intermediate schema holds a `match` on the dropped discriminator. Both ignored tests run.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qlaws track.

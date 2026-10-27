---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 862
summary: "two new merge refusals and the install package parse failure have no test, and the schema walk never draws a check on one constructor"
closed: 2026-10-27
tags:
  - quality
  - lang
  - test
---

# New merge refusals and the install parse failure are untested

## Report

No test reaches these messages:

- `crates/adtb-lang/src/evolve/ctor.rs:1591`: "check `{}` has no image under the merge; drop or rewrite it".
- `crates/adtb-lang/src/evolve/ctor.rs:1699`: "its constructors' field `{name}` goes to different fields".
- `crates/adtb-lang/src/evolve/mod.rs:1137-1146`: "the package `{from}` does not parse". The bad-package test at `evolve_tests.rs:387-396` fails on a check error ("nosuch"), not a parse error.

The walk's `add_check` (`crates/adtb-testkit/src/cases/walk.rs:1265`) draws only whole-relation checks, so the merge's check carrying (`merged_checks`, `carried_predicate`) is pinned only by two unit tests (`evolve_tests.rs:1397`, `:1427`).

Fix: a refusal test per message, and a walk draw of a check over one constructor.

## 2026-10-27: done

Refusal tests in `crates/adtb-lang/src/evolve_tests.rs`: `a_merge_refuses_a_check_over_a_merged_and_a_kept_constructor`, `a_merge_refuses_a_check_whose_field_goes_to_different_fields`, and an unparsable package in the install test. The walk draws checks over one constructor (`world::Check`, half of a sum's `add_check` draws), lets a merge take such a constructor, carries the check to the product, and offers a merge as a follow-up when such a check exists. `walks_merge_a_constructor_a_check_ranges_over` and two sweeps over walks of only checks and merges (`merges_carrying_checks_keep_engine_equal_to_reference`, `merges_carrying_checks_keep_their_laws`) in `crates/adtb-testkit/tests/differential.rs`.

Open: a revert of a merge that carried a check is refused ("no `provider` in scope"): the revert leaves the carried `match` on the dropped discriminator. `a_revert_of_a_merge_restores_the_check_it_carried` and `merges_carrying_checks_keep_their_laws` fail on it.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qtools track.

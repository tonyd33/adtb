---
created-at: 2026-11-17
status: open
kind: feature
priority: 30
tags: [test]
summary: "the slowest tests and the damage profile cost most of check's test time; rewrite them to run much faster with the same or more coverage"
---

# Feature: Test time per coverage

## Request

Measured on main 2026-11-17, alone, after an edit to adtb (work/briefs/measure-check-2026-11-17.md; per-test durations in nextest JUnit output):

- Suite: 3885 tests, 2559 s of test time, 190 s wall on 16 cores. 31 tests over 10 s take 652 s (25%). By crate: adtb-sim 1186 s, adtb 944, adtb-lang 168. Slowest: evolve laws `every_invertible_operator_and_its_revert_restore_the_source_before_it` 54.5 s, translate `a_pinned_insert_translates_across_every_generated_history_of_two_relations` 44.1 s, server bundles `each_binder_with_each_access_of_tone_is_stale_after_enable` 43.3 s and `a_conflict_branch_naming_old_reads_the_relation_unless_new_constructors_are_hidden` 43.1 s, `scripts::bundles` 39.4 s, five write_tests reference-agreement sweeps at 26 to 27 s each.
- Damage profile: 14 tests, 773 s of test time, 256 s wall. `a_torn_write_is_restored_from_its_image` alone takes 205.8 s and sets the wall; then 118.8, 96.8, 91.3, 75.9, 65.7, 56.7 s.

Maintainer, 2026-11-17: "It's hard to believe that there isn't a way we can write the tests that's much faster and still gets the same amount or even more coverage".

## Proposal

For each slow test, find where its time goes before changing it (profile one run), then rewrite it so it covers the same cases or more in much less time. Coverage is stated per test as the cases it reaches (seeds, fault points, operators, histories) and counted before and after; a rewrite that reaches fewer is not accepted. Candidates to check, none measured yet: a search that replays a whole workload from the start for each fault point or crash image where a snapshot or fork of the simulator state would replay once; sweeps that rebuild a schema, catalog or engine per case where one build serves all; a case split across parallel test functions where one long serial loop sets a stage's wall (the damage profile); generated inputs drawn more densely (shrinking, pairwise) instead of more seeds.

Exit: the damage profile's wall near its second-longest test or below, the suite's tests over 10 s cut by half or more, coverage counts equal or higher. After the tooling tracks.

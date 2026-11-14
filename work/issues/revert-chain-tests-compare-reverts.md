---
created-at: 2026-11-13
status: open
kind: bug
priority: 897
summary: "the revert chain tests compare one revert's result with another's, never with the source before the migration their name says is restored"
tags:
  - lang
  - test
  - quality
  - class-weak-tests
---

# The revert chain tests compare two reverts with each other

## Report

`a_revert_of_a_chain_of_reverts_restores_the_source_before_its_migration` (crates/adtb-lang/src/evolve_revert_tests.rs:485) applies a case's first migration, then `revert 0001`, `revert 0002` and `revert 0003`, and asserts `restored(&s[2]) == restored(&s[4])`. Both sides are results of a revert, so one wrong inverse that both chains share passes. `s[0]`, the source before the migration that the name says is restored, is never compared. `a_revert_of_a_revert_of_a_revert_applies_as_its_inverse` (evolve_revert_tests.rs:450) has the same shape: `s[2]` against `s[4]` and `s[3]` against `s[5]`.

The law search compares a single revert with the source before it, but not a chain, and not these `CASES` firsts unless the search draws the same operator.

Separately, `every_known_row_names_a_ticket` (crates/adtb-lang/src/evolve_law_tests.rs:1872) checks that each `KNOWN` row's ticket file exists, not that the ticket is open. A row whose ticket was resolved without the row changing still passes.

Found by reading; not reproduced.

Fix direction: assert `s[4]` against `s[0]` and `s[5]` against `s[1]` through the same comparison the law search uses, allowing only the documented difference (a constructor a revert adds back is new, declared at the revert's version). Have the `KNOWN` check also read each ticket's status.

Defect class: class-weak-tests

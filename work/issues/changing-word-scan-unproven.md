---
created-at: 2026-11-11
status: open
kind: bug
priority: 470
summary: "the reindex crash tests check the catalog root's changing word with a byte scan for u64::MAX - 1 that no test shows finds a set word, so their cleared-word assertions pass vacuously"
tags:
  - quality
  - engine
  - sim
  - test
  - class-weak-tests
---

# The reindex crash tests' changing-word check is never shown to find a set word

## Report

`changing_words` (crates/adtb-sim/tests/maintenance/catalog_root.rs:8) counts the 8-byte words of the catalog root page equal to `u64::MAX - 1` instead of decoding the root's changing field (catalog/root.rs). Its callers now assert only `0`: `a_panic_in_each_phase_of_the_job_ends_the_run_and_the_next_open_keeps_every_row` (maintenance/reindex.rs:197) and `a_panic_inside_the_swap_ends_the_run_and_the_next_open_leaves_a_clean_file` (maintenance/reindex_review.rs:1841).

The one assertion that the scan finds a set word, `assert_eq!(1, changing_words(&db), "changing stays set")` in `a_panic_inside_the_swap_frees_neither_tree_until_the_next_open`, went when that test became a crash test. So the `0` checks pass if the word moves to an offset the scan skips, changes encoding, or was never set in the phase under test, and "the next open clears the changing word" is unproven for each phase.

Direction: expose the decoded changing state of the catalog root through `testing`; assert it reads a reindex under way inside the phase hook before the injected panic, and none after the reopen.

Defect class: Tests asserting less than they claim.

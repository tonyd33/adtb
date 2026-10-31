---
created-at: 2026-10-31
status: open
kind: bug
priority: 400
summary: "tests that cannot fail, read refusals coarsely or are not run by check"
---

# Test audit findings

## Report

Read-only audit of all test code, 2026-10-31. Nothing was run, so each finding is a candidate and mutation is the measure. Paths are under crates/ unless they start otherwise. Coverage was partial: 1826 adtb-sim tests were largely grepped, and permission/tests, schema_tests and resolved_tests in adtb-lang were grepped only. Findings there are a floor.

This comes before fault-matrix, whose sweeps need floors. Work it by mechanism, in this order, and fix the instances through each mechanism. slot-test items wait for queue-tooling to land, and the line numbers need rechecking on the current tree.

## 1. Connect checks to their gate

- `check` runs neither `slot-test` nor `house-check-test`, and CI names neither. Add both, `slot-test --shell` if the full run costs too much. Add a test that every `work/tools/*-test` is named in `check`.
- devenv.nix: `suite --no-tests=pass`: zero tests pass. Drop the flag.
- work/tools/size-check.sh scans crates only: add apps.
- work/tools/tree-sitter-check: a stale `differential-allowlist` class is never reported. Dead guard at :126.
- work/tools/conf:75 misses a nonzero engine status after `adtb-conformance-done`.
- Fail open: workspace-gc:148 (failed clean exits 0), workspace-gc:59-60 (no files reads as idle), common.bash:41-42 (`pids_using` failure reads as unused), quality-metrics.sh:49-52.
- adtb-manual/build.rs:14: a `rust` block on a page outside `PAGES` never runs.

## 2. Sweeps prove they ran

`#[must_use]` on `Tally` and crash loop returns, and a floor helper of the shape at adtb-sim/tests/isolation.rs:203.

- adtb-sim/tests/pager.rs:715 `every_fault_at_once` and :942: `Tally` discarded, and with `damage_allowed` an open that always fails passes. Read in code by the lead.
- adtb-sim/tests/quarantine.rs:2286, server_binding.rs:1657, truncated_open.rs:168 (suspected), reindex_review.rs:419 (suspected), vacuum_threads.rs:691.
- adtb-testkit/tests/differential.rs:635,647 (no floor on partitions), front_fuzz.rs:93 (loops only over kinds present).
- adtb-lang/tests/spec_corpus.rs:54,85: `prelude_source_matches_the_spec` compares nothing on a parse failure. Read in code by the lead. Also :127, :28, :448.
- adtb/src/plan/tests/mod.rs:179,198,211, adtb/src/heap/tests.rs:195, adtb-lang/src/evolve_grant_tests.rs:1611, evolve_memo_tests.rs:731, adtb-sys/src/pages/loom_tests.rs (branches with no reach count, suspected).
- adtb-heap-cap/tests/cap.rs:133,164 (`--exact child_entry` passes on zero tests), :153 (proves nothing under a hard limit of 0), apps/campaign/tests/live_retry.rs:91, adtb-isolation serial.rs:113 and scenarios.rs:60 (suspected).

## 3. Refusals assert their reason

A `refused_with(reason)` helper, `#[must_use]` on `fails`, and a house-check flag on `assert!(... .is_err()` in added test lines. A helper that asserts status by default for shell runs.

- adtb-sim/tests: oplog.rs:1479,1487, server_adhoc.rs:1010,251,1693, lost_bitmap.rs:318,651, write_states.rs:1594, grant_rows.rs:1147, server_binding.rs:1829,1922, quarantine.rs:149, reindex_review.rs:966,1016,1040,181, reindex.rs:823.
- adtb-conformance/tests/engine_runner.rs:59,102,115. adtb/src/api/server/tests.rs:591,328,752. adtb/src/catalog/tests.rs:478,488. adtb/src/btree/tests.rs:48,159. adtb-lang/src/check/tests/writes.rs:220,226,245, mod.rs:511 (needle "expected").
- adtb-shell/tests/shell.rs:256,338,351,606,638. adtb-codegen-tests calls.rs:496. adtb-testkit/tests/differential.rs:591.

## 4. Names claim only what runs

- A: fix the test. adtb/src/storage/tests.rs:20 (one write, no older checkpoint); adtb/src/heap/tests.rs:130 (compacts a page with no hole); adtb/src/btree/tests.rs:209; adtb/src/plan/tests/mod.rs:399,421,1072 (plan never read); adtb/src/exec/tests.rs:124; adtb/src/write/verify.rs `every_other_problem_is_a_finding`; adtb-sys/src/crc32c.rs:211; adtb-sys/src/pages/loom_tests.rs:334 (accepts a lost fill); adtb-sys/src/os/tests.rs:58; adtb-lang/src/parse/tests.rs:457; adtb-lang/src/calendar.rs `civil_round_trips`; adtb-sim/tests/server_adhoc.rs:2088 (limit removal passes); scheduler.rs:158,110; embedding.rs:374; adtb-reference unicode.rs:188; adtb-shell/tests/shell.rs:266,463; calls.rs:398.
- B: delete, once what the audit names as covering it is confirmed. adtb/src/storage/tests.rs:426; adtb/src/key/tests.rs:1207 (never calls the encoder, read in code by the lead), :1218 second half, :1034; adtb/src/catalog/tests.rs:322; adtb-lang/src/check/tests/mod.rs:1486; evolve_memo_tests.rs:818; adtb-sys/src/pages/mod.rs `writers_shared_latches_...` (check Miri first); apps/campaign/tests/front_end.rs:345,355,159; engine.rs:318,890,895; adtb-shell/tests/shell.rs:533.
- C: pins of no contract. adtb-sim/tests/crash_images.rs digests include trace wording: state a contract or remove, after reading past :310. adtb-reference internal.rs message equality. adtb/src/catalog/tests.rs:78,152,160 literals. adtb-lang/src/check/tests/adhoc.rs:811,820 wall clock in a debug build.

## 5. Replace confirming tests by searches

Later, inside the mechanism tracks: sequences against a model of crashes; flip any byte of any log block; random grant, revoke and drop against a model of `require`; every Op crossed with every Val class; generated joins for plan_bounded; a translation agrees or refuses; concurrent `next_turn` calls under sim seeds.

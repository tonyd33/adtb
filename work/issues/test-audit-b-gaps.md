---
created-at: 2026-10-31
status: open
kind: bug
priority: 403
summary: "Gaps left by test-audit-b: house-check misses nested asserts, must_use is partial, stale names and half-done digests"
---

# Test audit b gaps

## Report

## Report

From the review of test-audit-b, by reading. Applies once that change lands. None is a wrong answer in the engine, so none held the track.

- work/tools/house-check `open_assert`: any added line that is only `)` closes tracking, so an `assert!(` holding a nested multi-line call before `.is_err()` passes. An added `.is_err()` under an `assert!(` context line passes too. A close on a context line survives across hunks and gives false positives. A text rule will keep having holes: prefer a clippy lint if one can see this.
- `#[must_use]` is off these helpers, which still have discards: inhabit.rs `fails` (36), server_binding `denied` (15+), checked_registration `denied` (6), server_authority `refused` (5), bundles `invalid` (2), registered `refused` (2), write_states `refused` (1), server_adhoc `detail`.
- adhoc.rs about line 797: stale doc `Returns how long f takes` on a test.
- catalog/tests.rs about line 149: the name claims keys then uniques, the body checks only dense ids.
- crash_images.rs about line 276: the segment case still folds the trace. `kept` and `full` run 4 seeds with no seed sensitivity. Goldens are regenerated values with no independent expected image.
- shell.rs `a_transact_block_refused_by_a_later_line_writes_nothing` is refused at check, so run-time rollback is not exercised.
- `refused_with` compares Display text, which cannot tell rejection variants apart.
- Not mutated: loom `a_page_filled_after_eviction_keeps_its_fill`, `a_dropped_transaction`, `a_transact_block`.

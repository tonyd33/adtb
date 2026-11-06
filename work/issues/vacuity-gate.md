---
created-at: 2026-10-31
status: resolved
kind: bug
priority: 403
summary: a test that cannot fail lands unless a reviewer happens to notice
closed: 2026-11-06
tags:
  - class-weak-tests
---

# Vacuity gate

## Report

About 20 tickets record tests asserting less than they claim. Hand mutants are listed in receipts, and `cargo-mutants` runs only at a milestone exit.

Add `cargo-mutants` to the dev shell and run it on the diff in the branch check, with a survivor needing a written reason. Measure its time first against a stop line in the brief. Add one helper asserting an exact refusal, and refuse `is_err` and substring refusal assertions in new tests.

Replaces the assertion gaps of bundles-test-assertions, command-test-assertions, embedding-test-assertions, file-scan-test-assertions, stale-value-declared-untested, uring-fallback-test-tautological going forward. Fixing those tests remains their own work.

## 2026-11-06: landed

`feat: refuse a change whose new code no test can fail`. `work/tools/vacuity` runs cargo-mutants on a track's diff, once per cargo workspace it touches, over the mutated packages, their direct workspace dependents with tests and adtb-sim in reach, under the machine lock, and writes `work/vacuity/<change id>.tsv`: a hash of the track's own change (context 1, hunk section text kept, line numbers and index lines dropped) and one line per mutant keyed by normalised file, function, mutation and ordinal, with a reason on every missed or timeout line; `none WORKSPACE FILES REASON` covers changed files with no mutant (test modules, declarations). `vacuity --check`, run by `land` and `check --changed`, fails on a missing record, a missing or stale line, an empty reason, a hash mismatch, a leftover in-place guard, or a changed `src/` file cargo-mutants does not know outside a `none` line. `adtb_testkit::refused_as` asserts a refusal value and its whole text; `work/tools/refusal-check` holds `is_err`/`is_ok`, `matches!(.., Err(_))`, bare `unwrap_err`, text `contains`/`starts_with` and `refused_with` assertions to a per-file allowlist. Two reviews found six blockers, all fixed (campaign workspace, outcomes untied to code, swallowed jj errors, unparseable patches, `#[path]` files, `none` across workspaces).

Accepted limits: a record is checked for shape and hash, not rerun, so a relabelled outcome passes; refusal-check counts per file and misses a predicate bound before the assert, `if .. panic!`, `Err(e)` in `matches!`, `format!` text and `let _ = unwrap_err()`. Measured cost: 3.5 to 4.5 min per viable mutant; mutation-schemata is the speedup.

## 2026-11-06: resolved

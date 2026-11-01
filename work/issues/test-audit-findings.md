---
created-at: 2026-10-31
status: in-progress
kind: bug
priority: 400
summary: "tests that cannot fail, read refusals coarsely or are not run by check"
branch: reference-fixtures
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

## From queue-tooling round 3

Whole-file `#![expect(clippy::disallowed_methods)]` in adtb-sim/tests/truncated_open.rs and reindex_review.rs, for thread spawns, would also let a bare `Command::new` through. Neither file spawns one today. Narrow them to the sites.

## slot-test is over-built

Ruled 2026-10-31, after test-audit-a lands. It grew from 351 to 532 lines in one day, most of the growth protecting the script from itself: variable and PATH scrubbing, closing file descriptors, a checkout snapshot, a re-exec, and tests of those. Replace with: `mktemp -d`, scratch `HOME`, `XDG_*`, `CARGO_HOME` and `TMPDIR`, copy the tools, `cd` there, remove `$tools` so cases use only each scratch repository's copies, one `trap rm`. Delete `snapshot`, `variables_leading_to`, `scratch_only_cases`, the PATH and variable scrub, the descriptor loop and the outer and inner stages. Keep the cases that prove `slot`, `land`, `workspace-gc` and the lock. Target: under the 351 it started from plus the lock and campaign cases.

## 2026-10-31: sections 1 and 2 landed

Dropped on reading: evolve_grant_tests.rs:1611, evolve_memo_tests.rs:731 (stale), scenarios.rs:60. Open: the slot-test simplification, sections 3 to 5, narrowing the whole-file expects.

## 2026-10-31: slot-test reduced

From 536 to 396 lines. Accepted: a future case could still type a real path.

## 2026-10-31: sections 3 and 4 ready to land

Candidate `test-audit-b`, tested revision
`480a4c3917d47183b82ae0e7b7909882bc399725`, base `fbb17db3`.
Independent review was already complete; no additional review round was
opened. Final full `check` passed: 3744 root tests, 48 campaign tests,
Loom, Miri, documentation, grammar, policy, size and tooling checks.
The source stayed unchanged across that run.

Two final-check repairs corrected independently derived expected
refusals: registration lists constructor, declared-key and projected
field grants; a fresh key with a duplicate named-unique value returns
that exact `Violated` report. The shared stress checker now matches
serialization by type. A deterministic invocation of the same
unique-refusal helper requires one fully validated report and exact
unchanged data. Temporarily restoring the wrong key-error validator
made that count fail (1 versus 0); source was restored afterward.

Initial timing samples differed by about 1 percent. One cached paired
root run under the same shared lock and four test slots measured main
214.043s and candidate 213.388s, both passing; campaign measured main
33.229s and candidate 32.817s. This establishes no observed default-run
increase in that pair, not a statistically proven speedup.

Next: land the candidate, run the post-land integration check, then
record its landed revision. Section 5 and the whole-file lint
expectations remain open; the enforcer/name/digest gaps are separately
tracked in test-audit-b-gaps. Do not close this umbrella ticket merely
because this slice lands.

## 2026-10-31: sections 3 and 4 landed

Landed as `36d64bd637f348f7a6b67577a23f884822706e8e` on main.
The post-land full `check` passed on an identical tree in
`audit-integration`: 3744 root tests, 48 campaign tests, 13 Loom tests
and 39 Miri tests, plus all configured static, documentation, grammar
and tooling checks. Root ran in 205.071s and campaign in 31.172s;
these are observations under that run's load, not a performance claim.

The quality-cadence review of the ten code commits through this slice
is complete. Its real new finding is tracked as
open-fallback-create-provenance; the final test-only refusal repairs
were independently derived, read by the lead and verified as above.
Section 5, the whole-file lint expectations and the separately filed
test-audit-b-gaps remain open.


## 2026-10-31: runtime text fixture pilot

Track `text-fixtures` starts from `ffd6c42d`. Migrate seven independently
reviewed pure reference-query text/result cases to runtime-loaded conformance
fixtures. Preserve original key kinds, explicit expected rows and paired
queries. Name retained coverage before removing each Rust wrapper; retain
internal, normalization and exact fault-payload assertions in Rust.
Acceptance includes all seven reached cases, a decisive join/dedup mutation,
and measured equivalent Rust-input versus runtime-fixture incremental edits.
Fixture-only edits must avoid Rust recompilation; no whole-suite speedup is
claimed from the pilot. One final full check follows fresh review and targeted
proof; unchanged landed inputs reuse that receipt. The umbrella remains open.


## 2026-10-31: runtime data is the default

The maintainer requires justification for compiled Rust cases. Separate a
shared Rust driver from case inputs and expectations: typed/internal assertions
can remain in the driver while the cases load at runtime. A runner's missing
feature does not alone justify retaining many wrappers; compare a small shared
extension with migration while preserving every relevant assertion.

The current seven-case pilot is being corrected to preserve reference-only
responsibility and allow corpus growth without Rust edits. A separate read-only
agent is mapping all remaining reference-query tests and the complete checker
root test module into fixture routes and explicitly justified retained Rust
families. This is a bounded next-batch proposal, not a completed suite audit or
a claim that every case can use the current conformance format unchanged.

## 2026-10-31: reference fixture pilot landed

Landed `69be892dc369` after independent review and a passing final check
on `30383e8f9263`. Seven runtime `.test` files under
`crates/adtb-conformance/tests/reference-queries` preserve seven original
reference-only cases and eight query outcomes; one shared runner replaces
67 Rust wrapper lines. Data-only eighth-file growth changed no executable
and its wrong expected row failed; a projection mutant failed the intended
assertion and exact restoration passed. No engine coverage was added.

The final check passed: 3754 root tests (eight skipped), 48 campaign tests
(one skipped), 13 Loom tests and 39 Miri tests (two ignored), plus all
configured lint, formatting, policy, documentation, grammar and tool checks.
The check's own exit was zero and before/after source states were identical.
Tested-to-landed differences were only reviewed ticket/index prose; source,
fixtures and configuration matched, so the receipt was reused without a
second full run.

One matched warm edit observed 1.162678 s Rust rebuild feedback versus
0.063578 s runtime-data feedback; this is no clean-build or whole-suite
speedup claim. Three paired process medians were 0.038100 s before and
0.066538 s after under differing harness contexts. The larger checker
migration is active in checker-runtime-fixtures. This umbrella remains open
for its unresolved audit findings.

## 2026-10-31: next reference runtime batch

Migrate the 16 mapped pure reference-query families into runtime .test data using the existing reference-only conformance runner. Preserve all source/outcome pairs, UUID and timestamp values, ordering and List container identity. Keep the other parameterized/typed/fault cases. Require data-only additions without recompilation, independent preservation review and bounded affected runtime cost; do not repeat expensive old/new warm builds just to fill a trial quota.

The two source tracks use separate files and warm workspaces. Authors own
execution and corrections; fresh independent reviews precede a lead-assigned
combined final check when both are ready. No duplicate full runs.

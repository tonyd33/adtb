---
created-at: 2026-10-31
status: in-progress
kind: bug
priority: 400
summary: "tests that cannot fail, read refusals coarsely or are not run by check"
branch: reference-write-outcomes
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

## 2026-10-31: continuing beyond the first fixture batches

The active 16-family reference batch is followed by the remaining checker
module: 48 adjacent cases and three structural/resource cases outside the
first 108-family migration. Reuse the existing driver for representable
inputs now; retain typed assertion logic and require explicit justification
for compiled case data or a proposed small probe extension. Preserve every
original assertion and data-only corpus growth. This is not a complete
audit of the wider Rust test inventory.

The other 22 reference-query cases need typed Fixture support for external
bindings, registered calls, fault payloads and raw identity assertions. They
remain queued for that adapter rather than being declared unmigratable.
The SQLite baseline is a parallel milestone deliverable; fixture migration
continues instead of stopping at these two modules.

## 2026-10-31: final verification boundary for fixture-only batch

The 16-family reference migration and 49-family checker data continuation
form a separate ready batch. Their changes are confined to test inputs,
assertion drivers and the conformance diagnostic format/probe adapter.
Database/language production files, dependency/build/feature configuration
and global containment are unchanged. Direct consumers are within adtb-lang,
adtb-reference, adtb-conformance, adtb-probe and the adtb-testkit soundness
test; run complete tests of all five packages on their final combined tree and reuse unaffected full-check
evidence from `6685d287`/tested `047020687851`. Existing strict affected
lints, preservation and mutation receipts remain required.

The production pinned-reader change is separate and still requires its
reviewed final full check. Its source-complexity correction does not gate
the unrelated fixture batch. This supersedes earlier broad full-check
wording for these bounded fixture-only changes.


## 2026-10-31: reference and checker continuation landed

Reference migration landed as `9b0e979b59c5`; checker continuation landed
as `c9f2035e4731`. Fresh independent preservation reviews accepted both.
The reference batch moves 16 additional families and 20 semantic outcomes
into two existing reference-only scripts. Together with the pilot, 23 of
45 mapped families now load ordinary case data at runtime. The remaining
22 need shared typed support for bindings, registered calls, fault payloads
and identity assertions; they are queued, not declared unmigratable.

The checker continuation externalizes data for 49 more families: 101 source
operations and 102 source/context strings. Across both checker batches,
157 of the original 159 families load ordinary case data at runtime.
Typed property assertions remain in reusable Rust drivers. The two retained
case-construction exceptions are the generated wide-stack resource test
and the synthetic shared kernel DAG. The corpus now has 492 runtime case
IDs; all 110 prior checker fixture files remained byte-identical.

Review found per-file validation outside failure accounting. The author
moved it inside the existing catch; the bad-file probe then reported its
named failure while all 464 valid ordinary cases still executed. Restored
healthy groups passed. Data-only additions and expected-result changes
proved discovery, assertion sensitivity and unchanged executable hashes.
These are local feedback proofs, not a clean-build or whole-suite speedup.

Final combined validation on `a0fe65d2013b` passed all 1169 selected tests
across complete adtb-lang, adtb-reference, adtb-conformance, adtb-probe and
adtb-testkit packages. Two existing deep-profile exclusions were unchanged.
Strict affected lints and static checks passed; suite and final testkit
clippy each returned zero. Revision, status and all 3152 tracked input
hashes were identical before and after. Tested-to-landed differences were
only reviewed ticket prose. The bounded fixture/tool verification policy
reuses prior full-check evidence only for unchanged components; that old
run did not test the new drivers. No duplicate full check was run.

This umbrella remains open for the queued migrations, the wider inventory,
section 5 and separately tracked audit gaps. Production pinned-reader work
is separate and did not delay this fixture delivery.


## 2026-10-31: next typed reference migration

Track `reference-typed-fixtures` owns the remaining 22 mapped reference-query
families. First assess a small shared Fixture driver against existing native
format and dependencies; preserve external parameters, registered calls,
exact typed faults and raw identity assertions. Ordinary input and expected
data default to runtime files; reusable typed assertions remain Rust.
No production API, dependency, build or engine-coverage expansion is assumed.
Independent preservation review and complete affected-package validation
follow a stable candidate; no duplicate full check is planned.


The driver assessment selected a test-local adapter around existing Fixture
APIs. Importing conformance would add engine dependencies; keep the reference
dependency graph unchanged. Typed evaluation/call modes preserve external
bindings and check/apply boundaries; narrow shared probes retain independent
raw constructor and exact fault expectations. List expectations preserve an
ordered element vector. Strict runtime discovery must reject unused inputs,
report each case failure and continue healthy cases before failing the run.
This bounded implementation is approved; it is not yet validated or landed.


## 2026-11-01: typed reference migration landed

Landed `7f4fef979082` after fresh independent preservation review.
The remaining 22 mapped reference-query families now run 59 runtime
source/outcome pairs. All 45 originally mapped reference-query families
have runtime case data across the three batches. The shared typed adapter
preserves external bindings, registered check/apply calls, exact Abort/Fault
payloads, independently supplied raw constructor IDs, and ordered List
values including duplicates. Reference dependencies and production libraries
are unchanged. Stronger independent complete rows replace pairwise/count
oracles without dropping original query sources or parameter values.

Final complete reference-package validation passed 217 tests across ten
binaries, zero skips, native exit zero. Revision, status and all 3179 tracked
file hash/metadata entries were identical before and after on tested
`1db6ae242d65`. Own command elapsed was 8.829 seconds including compilation;
nextest execution was 1.621 seconds. Affected strict lint and static checks
passed. Tested-to-landed changes were only previously reviewed ticket/index
prose; the passing receipt is reused without another engine-wide check.
Previous full receipts apply only to unchanged components, not this driver.

Data-only valid additions ran without executable hash/mtime changes; bad
outcomes, missing originals, unused fields, unknown files/modes, raw IDs,
fault and List expectations failed at their intended boundaries. An actual
Rust driver binding mutation also failed exact outcomes; it was not a
production interpreter mutant. Restored healthy checks passed. One matched
warm feedback observation was 1.096 seconds for a compiled binding edit
versus 0.464 seconds for runtime expected data, including command overhead.
No clean-build or whole-suite saving is claimed.

The migration of these two modules is complete; the wider Rust test inventory,
section 5 and separately tracked quality gaps remain open. Select the next
bounded data-migration batch from that inventory rather than treating these
modules as the end of the work.


## 2026-11-01: reference-write continuation

A body-level inventory covered all 78 current reference-write families and
their helpers: 107 statement outcomes, 41 assertion queries and 18 native
table-count invocations. This is source evidence, not a current timing run.
The existing Fixture also compares original and normalized whole outcomes
and checks unchanged database state after refusal. Native conformance alone
would lose those boundaries; use a small mutable typed adapter.

Track `reference-writes-fixtures` starts with seven insert/combine families:
existing equal-key refusal, constructor insert, query-driven constructor
insert, sum insert, conflicting duplicate insert, conflicting duplicate
update and equal duplicate update. Preserve nine statement outcomes, five
exact probes, external bindings and all normalization/refusal-state checks;
leave the other 71 wrappers unchanged. Load the original setup through
ordered runtime row loads, with a fresh Fixture per family. Setup duplication
is explicit; the shared compiled fixture for other consumers is unchanged.
Builds wait for a free slot while source/data implementation proceeds.
Independent preservation review, same-executable edit/addition proof and
complete affected-package validation precede landing. No new framework,
dependency, production API or engine coverage is part of this batch.


## 2026-11-01: seven reference-write families landed

Landed `585dfa3a7e42` after fresh independent preservation review. Seven
families now load nine statement outcomes and five probes from runtime data.
The other 71 wrappers are byte-identical. A 92-line reference-local extraction
shares envelope/value/setup mechanics with the query runner; all 59 query
cases and their assertions are unchanged. The mutable runner retains whole
original/normalized Outcome equality, exact typed KeyViolation relation/key,
post-refusal database equality and singleton cardinality checks.

The final complete reference-package suite on `69923802e094` passed all 211
tests across ten binaries with zero skips and native exit zero. Before/after
revision, status and all 3192 tracked file hash/metadata entries matched.
Command time was 8.132 seconds including compilation; test execution 1.617
seconds. Strict reference lint, formatting, exact-range house and size checks
passed. Landing changed only separately reviewed ticket/index prose, so the
passing receipt is reused without another engine-wide run.

Runtime edits/additions kept executable hash and mtime unchanged; omitted
steps, exact wrong outcomes, duplicate/unknown fields and newly added bad
cases failed by name. Actual temporary Fixture bridge perturbations proved
normalization and post-refusal state assertions still execute; both were
restored exactly. Those are bridge witnesses, not production mutants.
One warm feedback pair observed 2.170 seconds for a compiled input edit and
0.732 seconds for runtime data, with equal test selection. No clean-build or
whole-suite saving is claimed. Runtime setup duplicates 114 lines beside the
unchanged shared compiled fixture; its broader compilation cost remains.

This batch is complete. The remaining 71 write families and the wider Rust
inventory remain open; their ordinary data is not declared unmigratable.
Use the retained complete family map to choose the next closed cohort.


## 2026-11-01: ordinary reference-write cohort extended

Landed `1dc91ac6a423` after independent preservation review. Another 23
families move 24 statement outcomes, 12 assertion queries and nine native
table counts into 45 runtime steps. Totals are 30 of 78 write families and
59 runtime steps; all 48 retained wrappers are byte-identical. The prior seven
families, setup/schema, query corpus and shared/testkit code remain unchanged.
Two small typed assertions compare successful Outcome.result and native table
cardinality. Exact Key(Int), Option, unit, UUID and binding values remain
native comparisons, with normalization and refusal-state checks preserved.

The complete reference-package run passed 188/188 tests across ten binaries,
zero skipped, native exit zero, in 2.415 seconds including the warm build
(1.602s execution). All 3237 combined tracked file/metadata entries, revision
and status were identical before and after on the exact landed revision.
All 27 reviewed paths matched after rebase onto the native-tool checkpoint;
other main paths were retained. Strict lint and static receipts apply to the
same source/configuration. No post-land rerun or full engine suite is needed.
New exact result/count bridge perturbations and data witnesses failed their
intended assertions, including unit comparison; restored healthy runs passed.
Runtime edit/addition kept executable hash and mtime unchanged. These do not
establish a clean-build or whole-suite speedup. The remaining 48 families and
wider inventory stay open for later typed fixture extensions.


## 2026-11-01: next refusal cohort assigned

Workspace `reference-writes-remaining`, based on `89891a340f30`, owns 18
remaining default-Fixture refusal families: fault, user error, malformed input,
key change, reference/unique/check/domain/transition refusal and report
projection. The planned replacement has 26 runtime steps; the previous 30
families and 59 steps remain unchanged. A bounded typed refusal helper keeps
whole original/normalized outcomes and complete post-refusal state checks.
The fixed try/fail case should assert its exact user message rather than add
a variant-only matching escape hatch. No production, shared setup, manifest,
dependency or containment change belongs to this track.

Acceptance: independent assertion-preservation review; sensitive new native
refusal/report/state bridge probes with exact restoration; data edit/addition
without relinking; affected strict lint/static checks; one complete
`work/tools/suite -p adtb-reference` run on the final reviewed combined inputs,
owned by the author, with native exit and unchanged tracked inputs/status.
Retain the fixture-verification boundary and earlier unchanged component
receipts. No new full engine or global binary-discovery run is warranted by
this scoped migration. The remaining families and broader inventory stay open.


## 2026-11-01: refusal fixture cohort landed

Landed `9bcfa6018d77` after fresh independent source and execution-evidence
review. Another 18 families preserve 23 statement outcomes, one scalar query
and two table counts in 26 runtime steps. Totals: 48 of 78 write families,
85 steps; all 30 retained wrappers and the previous 30 runtime families remain
unchanged. The new 128-line typed refusal helper keeps exact native faults,
rejection payloads, report counts/projections, external parameters and complete
post-refusal state. The fixed try/fail case now checks its exact message.

Five compiled assertion-bridge mutants failed at their intended assertions;
eleven runtime-data outcomes proved error checks, strict field/step handling
and case growth without relinking. Exact restoration preceded the final
complete reference-package run: 170/170, ten binaries, zero skipped, native
exit zero, 1.588s execution and 3.201s command elapsed. All 3256 tracked input/
metadata entries, revision and status were identical before and after. The
landed tree equals the tested tree; no full engine/discovery or post-land
repeat ran. Lint, formatting, exact-range house and size checks passed.

The batch took approximately seven minutes from local brief creation to
landing; that is an observed whole-track interval, not measured coding time
or a general speedup. The remaining 30 families need bounded drivers for
successful change/cross-result assertions, native stage/kernel hooks, custom
setup, row limits and related supplies; they remain runtime-data candidates.
The umbrella stays open and no defect class is declared closed.


## 2026-11-01: successful-outcome cohort assigned

Workspace `reference-write-outcomes`, based on `7e309deee2eb`, retains the
same author for 12 more families: successful change-map assertions, fixed clock
results, raw constructor/field probes, generated-key exclusion and a key-upsert
already supported by the existing driver. Keep all 48 prior runtime families
and their 85 steps, setup/shared inputs and 18 retained wrappers unchanged.
Use small typed Outcome and probe helpers; no capture interpreter or new
production/dependency/configuration boundary. The existing query-state test
compares its result to the fixture after assignment and cannot detect a change.
Replace that tautology with independent pre-query state equality and a mutation
of the actual state-comparison boundary. Preserve original native/raw assertions
and whole normalization outcomes; ordinary case data remains runtime loaded.

Acceptance follows the bounded fixture rule: fresh independent preservation
review overlapping new sensitivity work; exact restoration, runtime addition/
edit without relinking, strict affected lint/static checks and one complete
reference-package final run with native exit and unchanged input/id/status
records. The author owns commands, correction and final verification; the lead
owns landing/tree comparison. No unchanged full engine or discovery rerun.

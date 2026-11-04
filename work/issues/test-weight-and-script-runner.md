---
created-at: 2026-11-03
status: open
kind: feature
priority: 150
summary: "3600 Rust #[test] functions in 117 test binaries; adtb-sim alone has 1828 tests in 64 binaries and 72k lines, dominating test compile time"
---

# Feature: Audit Rust test weight and move scripted tests to a runner

## Request

Maintainer, 2026-11-03: the test count deserves an audit for value, and tests that are scripts should run from data through a runner, since Rust tests drive compile time. A new runner is preferred over stretching an existing one.

Counts on 2026-11-03: adtb-sim 1828 tests, 64 binaries, about 72k lines; adtb-lang 790; adtb 393 unit tests; 117 test binaries in all. Existing runners: conformance scripts (`conformance/*.test`, reference and engine), checker fixtures (crates/adtb-lang/tests/checker), the spec corpus, adtb-testkit sections.rs, the simkit crash and failure search.

## Proposal

1. Read-only audit of adtb-sim: each test is one of: fits a conformance script; fits an engine-only script runner (migrate, reopen, pin and call a bundle, act as a role, through a server, exact refusal value); stays Rust (crash and fault searches, simulator internals, private units); low value (covered by a stronger test, a getter, an unstated detail). Measure test build time per binary today. Output: the runner's directives and the count per bucket.
2. Merge adtb-sim's 64 test binaries into one, with no other change.
3. Build the runner; move tests in batches, each batch deleting the Rust tests it replaces and naming retained coverage.

## Open questions

## 2026-11-03: audit

Read-only audit of crates/adtb-sim/tests, bucketed from names, helpers and assertion markers with about 60 bodies read whole; mixed files carry about 10% error. 1828 tests: 60 fit conformance scripts (constraints 28, files 25, api 6, principal 1); 714 fit an engine-only runner (bundles 151, oplog 74, server_adhoc 68, server_binding 50, server_authority 42, registered 30, checked_registration 29, api 29, server_commands 28, commands 27, past 25, others); 1050 stay Rust (searches, faults, internals, interleavings); 4 are low value. 13 tests assert only `is_err()`.

Runner: eleven directives cover 588 of the 714: `migrate`, an exact `error C VALUE` comparing the whole `RejectReason` in textual syntax, `bundle`/`register`, `as ROLE`, `reopen`, `call`, an engine-only catalog query, open options, named transactions, `advance` on the simulated clock, host functions from a fixed registry. Shared schemas are fixture files pulled in by `use NAME`. The runner never stands in for the reference, does not live in `conformance/`, and runs no searches.

Low value: `exec::bench_scans_joins_and_groups` (ignored, asserts nothing); `oplog::a_second_migration_declaring_a_relation_applies_at_open` and `api::a_second_migration_applies_at_open` (covered by `oplog::migrations_apply_in_order_each_producing_a_version_that_survives_reopening`); `quarantine::a_file_with_a_damaged_catalog_page_opens_for_verify` overlaps open_damage but is the only test of the `Checksum` finding for that page, so it stays until that is covered elsewhere.

Batches: 0 delete the low value tests; 1 the 60 into `conformance/`; 2 runner core with oplog, longkey, commands, api, past (about 170 tests); 3 bundles (about 234); 4 roles and server (about 230); 5 the rest (about 80). In all about 20.5k Rust lines out, 8.45k script lines in.

Merging the 64 binaries: test names gain a module path, so every `test(=NAME)` filter in .config/nextest.toml (six `threads-required` overrides, the server_adhoc slots-2 group keyed on `binary(=server_adhoc)`) and crates/adtb-heap-cap/tests/cap.rs change in the same commit; server_adhoc reruns itself with `--exact NAME`; three names repeat across files; `#[path]` modules in constraints.rs and io_contract.rs; file-level `#![expect]` in reindex_review.rs and truncated_open.rs; scratch directory names under `CARGO_TARGET_TMPDIR` need a clash check. Build time is not yet measured.

## 2026-11-03: steps 1 and 2 landed

`test: drop three adtb-sim tests a stronger test covers` (the bench, and the two second-migration tests covered by `oplog::migrations_apply_in_order_each_producing_a_version_that_survives_reopening`; the quarantine test stays). `test: build adtb-sim's tests as one binary`: tests/sim/main.rs with one module per former file; nextest filters, the heap-cap scan (`module_prefix`) and server_adhoc's `--exact` reruns carry module paths; `every_source_of_the_binary_is_a_module` fails naming any file under tests/sim no `mod` line reaches. Test names before and after equal as `BINARY::NAME` against `MODULE::NAME`, less the three deleted. Measured once at 3 build jobs: clean build of adtb-sim tests 190 s to 256 s; one line in crates/adtb/src 17.9 s to 10.2 s; one test file edited 0.8 s to about 6 s; test binaries 2.07 GB to 88 MB. Full check 3677 passed.

Next: step 3, the engine script runner and its first batch (oplog, longkey, commands, api, past).

## 2026-11-03: runner and oplog batch landed

`test: run engine scripts and move the oplog tests onto them`. Runner: crates/adtb-sim/tests/sim/scripts/ (mod.rs, read.rs, session.rs), one test per area under crates/adtb-sim/scripts/AREA/, fixtures in scripts/fixtures/, `work/tools/script PATH...` by hand. It shares adtb-conformance's reader (`blocks`, `engine::directive`, `engine::ended`) and never runs the reference. Directives: `seed`, `use`, `option`, `migrate [at V] [refused VALUE]`, `reopen [refused VALUE]`, `role`, `run`, `query [rowsort]`, `error VALUE` comparing the whole refusal. A script fails on an unknown directive, a step with no open database, `at V` past the log, no directive but `seed`/`use`, a `use` cycle; an area fails on no scripts, an unreadable directory or a stray file; `every_area_has_its_test_holding_its_slots` ties each area test to its nextest slots; probes in scripts/probes/ with `.differs` files pin each refusal. 73 oplog tests moved into 77 scripts with the same or stronger assertions, 8 kept in Rust; oplog.rs 1801 to 360 lines. Review found no blockers; one delta closed every way a script could pass without checking. Full check 3611 passed.

Next batches need `bundle`/`register`, `call`, `as ROLE`, named transactions, `advance`, `host`: batch 3 bundles, batch 4 roles and server, then commands, api, past, longkey, and the 60 conformance-fit tests into `conformance/`.

## 2026-11-03: bundles batch landed

`test: move the bundle tests onto engine scripts`. 119 bundle tests moved into 131 scripts in crates/adtb-sim/scripts/bundles/, 41 kept in Rust (crash tests, admission with cancel, deadlines and interactive transactions, sweeps and searches, 16 host-function tests waiting on a `host` directive); bundles.rs 6298 to 3126 lines. New directives: `log`, `bundle HEADER [use NAME] [refused VALUE]`, `call NAME [ORDINAL [committed]] [rowsort] [error VALUE]` with set results compared as sets as conformance.md states, `advance`, bodiless `migrate at V`. The three `is_err()` assertions are exact refusals. Review found no blockers; one delta. Full check passed.

Known pin: one committed call's value holds `identities.key(65)`, the key a sequence hands out after a reopen, which no contract states. Dropped: the BundleExpired message text, which no contract states.

Next: `host` and the 16 host-function tests, `as ROLE` and the server batch, then commands, api, past, longkey, and the 60 conformance-fit tests.

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

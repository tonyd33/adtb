---
created-at: 2026-11-06
status: open
kind: feature
priority: 100
summary: "a targeted adtb-sim run after an edit rebuilds the test binary in a median 114-124 s, 80% of nextest time and 38% of author time on today's tracks; build jobs past 3 do not shorten it"
---

# Feature: Cut the adtb-sim rebuild after an edit

## Request

## Proposal

## Open questions

## 2026-11-06

From the timing study of 2026-11-06 (work/briefs/review-devex3.md, local): across 89 nextest logs 80% of the time is compile; a rebuild of the adtb-sim test binary after an edit has a median of 114-124 s at 6 jobs alone, the same 114 s at 3 jobs, 174 s at 3 jobs beside another track. It was 50 to 123 min per author round on the four engine tracks. Measure where a rebuild goes (the adtb crate, adtb-sim's test crate, the link) and try the cheap levers first: a faster linker (mold or lld), codegen units and incremental settings for the test profile, splitting the slowest modules out of the adtb crate's rebuild path. Report the rebuild time per lever.

## 2026-11-06: prioritized

Maintainer, 2026-11-06: prioritize this. Next track to dispatch.

## 2026-11-06: where the rebuild goes, and the levers tried

Measured with `cargo test --no-run -p adtb-sim --timings` (the test profile, opt-level 1), baseline and lever back to back, three rounds, under the load of two other tracks. An edit to a hot generic engine function (`Pager::shared`): 98 to 167 s; an edit to one sim test file: about 8 s. Per unit: adtb 18 to 28 s (codegen 15 to 24), the adtb-sim lib about 9 s, the `sim` test unit 80 to 139 s, starting only after adtb, link under 2 s. The `sim` frontend is 2 s warm, so the cost is the codegen of the one `sim` test crate (85 modules, 70k lines, one rustc unit), which instantiates the engine's generics itself: generics are shared across crates by default only at opt-level 0.

Levers, none kept: the linker is already lld (link under 2 s); test-profile opt-level 0 rebuilds in 13 to 24 s but runs the suite 3.4 times slower with 22 failures and 14 timeouts; opt-level 0 for adtb-sim alone rebuilds in 25 to 28 s, runs adtb-sim and adtb 20 to 30% slower (161 s to 195 s) with a stack overflow in `server_adhoc::fused_loops_under_the_run_bound_run_on_the_reduced_stack` and two `bundles::` timeouts; `CARGO_INCREMENTAL=0` and `codegen-units=16` are slower, the latter turning an 8 s test-file edit into 18 to 27 s.

Next: split the `sim` test binary by area, so a targeted run builds one smaller unit and a full build codegens them in parallel.

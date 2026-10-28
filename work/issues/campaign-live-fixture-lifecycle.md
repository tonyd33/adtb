---
created-at: 2026-10-28
status: resolved
kind: bug
priority: 849
summary: the live concurrency test leaks an Underway encounter into the following scenario
closed: 2026-10-28
tags:
  - quality
  - test
---

# Campaign live fixture lifecycle

## Report

Exact frozen principal fullcheck f987476f/base928b7d38 returned100 with all
2,387engine tests passed and46/47campaign tests passed. Existing
live::concurrent_planning_starts_and_cross_path_refusals failed at
tests/live/atomic.rs41 while starting the second fixture encounter. Typed
constraint campaign::encounters.unique(session) in Underway reported keys65/66
for session2. The report includes a proposed row; it does not prove two Underway
rows committed. The engine correctly refuses the second proposed Underway.

Root and author independently traced live.rs373-378: it invokes the roster/start
race helper then the stale-roster/lifecycle helper on the SAME live session.
The first helper creates65. Start-win leaves it Underway after assertions;
add-win leaves it Prepared. The next helper creates66 and always starts it.
Thus one allowed native schedule leaves a conflicting active encounter behind.
No deterministic seed applies to this OS-thread race. This is a test scenario
lifecycle defect, not a reproduced engine invariant violation.

Fresh campaign-fixture workspace on907c5a44 owns only tests/live/atomic.rs and
necessary test caller changes. Preserve the real Barrier race and exact typed
assertions; deterministically exercise both allowed results and settle/isolate
each scenario before another uses that session. No forced original winner,
application locks/retries, error swallowing, lax oracle, production/bundle/schema
or engine changes. Lead reviews and lands this first, runs full maincheck, then
principal rebases and reruns its complete warm-cache check. Author owns targeted
failing case, fullcampaign suite and strict campaignclippy/fmt/house; broader
root/docs check explicitly belongs to lead afterlanding. Measure defaultcost.

Original failed fullcheck:406.417s,16:36:57Z–16:43:43Z, input/config hashes and
revision stable; root2387/8skipped51.146s, campaign46pass/1fail/1skip20.809s.
Later policy/size/docs/house unfinished. Evidence original sibling
adtb-principal-validation/target/principal-full/0.log206-225 and receipt remain;
no rerun erases it. New exact-source receipts/tests must establish the correction.

## 2026-10-28: resolved

Landedfda78edb, test(campaign): isolate encounter race fixtures. Reviewed source
98163a50 on907c5a44 changes only atomic.rs. Original Barrier race and exact typed
rejection/roster/tracker assertions remain. Additional start-first/add-first
controls guarantee both legal outcomes are exercised. Add-win uses one new start
with the complete current roster; each case ends Resolved/no tracker, then starts
and ends a following encounter in the SAME session. No production, engine, schema,
bundle, lock/retry/error suppression or forced native winner.

Author targeted1pass5.742s; fullcampaign47pass/1skip21.558s (live8.868s); strict
campaignclippy/fmt/house all0. All2,551tracked hashes stable after each stage;
root localCargo configs absent, existing appconfig is included in manifest.
No isolated helper-only or cross-condition timingdelta claimed. Fresh independent
review rehashed all inputs and found no blocker; scopedhouse0.

Lead main fullcheck passed0 on e961f8ee0154e01434bc513c44a0fe1d7fcbc61c, parent
fda78edb,16:58:02Z–16:59:37Z,95s. Exact command from root: direnv exec
/home/tony/code/adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check.
All tracked/local root+app config hashes and revision unchanged; Rust1.98.1/
Cargo1.98.0, configured defaults; permission/queue/execution-onlyunknown.
Root2,373tests/8skipped48.486s; campaign47/1skipped21.315s, live8.269s; all lint,
dependency,size/docs passed. Docs10tests/420syntax/61pages/6730links/4995fragments.
Main log target/campaign-fixture-integration/check.log SHA256
62ba0266f1c184a077c97589a7cb96e780de7cfda29ae50ad8c79002119fbf7e.

Receipts/review in work/briefs/receipt-campaign-fixture.md,
receipt-campaign-fixture-main.md and review-campaign-fixture.md. Source/logs
retained;8files+68archivedfixtures SHA256preserved in target/campaign-fixture-
evidence before completeddebugcleanup. Original failed principal full100 remains
preserved; its source was not blamed for this independent lifecycle leak.
Principal rebased cleanly2de0c4f9 onfda78edb and now reruns full verification.

## 2026-10-28: resolved

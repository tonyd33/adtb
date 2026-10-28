---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 849
summary: the live concurrency test leaks an Underway encounter into the following scenario
branch: campaign-fixture
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

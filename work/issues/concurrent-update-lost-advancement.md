---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 698
summary: two successful concurrent registered updates may persist only one advancement
branch: engine-update-loss
tags:
  - engine
  - test
---

# Two successful concurrent updates may lose an advancement

## Report

Campaign-live snapshot 3c5f39eb57ae41e5442f81f8e445883702ad714e,
apps/campaign/tests/live.rs:201, live_flow_concurrent_turns_reopen_and_player_secrets,
failed during the full campaign run. Two concurrent next_turn HTTP requests
both returned 303, but after close/reopen a two-combatant encounter was at
(round,turn)=(1,2), expected (2,1). Earlier isolated run passed. Suspected
engine defect, not yet established by a direct-call reproduction. Do not
treat it as diagnosed or add an application workaround.

next_turn is unchanged from prior bundle1.5: update encounters.Underway,
advance to the minimum higher initiative slot, otherwise increment round
and select the least slot. Two committed serializable calls on initial
(1,1) with slots1,2 must finish at (2,1). A retry/refusal must be visible,
never successful loss of an advancement. Check state before/after reopen.

Evidence retained in /home/tony/code/adtb-campaign-live/target/
live-all-campaign.log (44 passed,1 failed,1 skipped; assertion left1,2 right2,1).
Current worker is reducing to typed engine calls, including positions/attempts
and persisted state. Application feature expansion and server-wire work paused
under user's engine-first rule. Preserve all failing tests/logs and source.

Next: confirm minimal engine-layer regression under crates/adtb-sim/tests;
dispatch one fresh engine workspace for root cause and principled fix. Keep
campaign failed run recorded even if isolated reruns pass. Lead owns review,
full branch/main checks and verified landing before feature work resumes.

## 2026-10-28: direct engine reproduction confirmed

Campaign-live snapshot 9135c16ed6385687b5d5de7497b6dba04c7aafe2 adds
tests/live_retry.rs. Two native threads Barrier-synchronize typed v1_5/schema5
next_turn calls, no HTTP or new bundle. Loop32 failed at iteration9: both
calls committed, epoch1 lsn8489756 and8489404, attempts2 and1; final(10,2),
expected(11,1), with both initiative slots present. Evidence:
/home/tony/code/adtb-campaign-live/target/live-direct-repro.log.

Engine-update-loss workspace starts at5320aaa7; fresh agent owns minimal
engine-layer reproduction, invariant diagnosis and principled fix. Brief
work/briefs/brief-engine-update-loss.md. Root cause not yet diagnosed; stale
retry computation is a hypothesis, not a finding. Application worker stopped
feature work and is writing final handoff. Server-wire paused with unverified
partial codec snapshot2950faa3; handoff-server-wire.md preserves next steps.

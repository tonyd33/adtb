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

## 2026-10-28: minimal counter reproduces

Engine workspace adds crates/adtb-sim/tests/update_concurrency.rs. Before any
fix, two concurrent registered counter increments reproduced the loss in
0.30 seconds at iteration21: both committed positions5348589/5348188,
attempts2/1, counter43 expected44. Log target/update-before2.log in sibling
engine-update-loss. This removes campaign, HTTP and turn aggregate dependencies.
Root cause remains under investigation; version claim/rollback ownership is
a hypothesis. Deterministic interleaving and general fix are next.

## 2026-10-28: diagnosis, repair and independent review

Trace confirmed a check/mutation race: writers38 and37 both passed writable
on the same original version. Writer38 installed a successor; writer37 then
overwrote its xmax/next. Writer38 committed,37 aborted, and retry39 missed the
committed successor. No stale retry cache was involved. Deterministic seed4,
switch_rate0.3, two threads of8 increments failed the baseline in0.172s with
two visible values[2,15] instead of singleton16. Retained evidence:
target/update-sim-baseline.{log,status} and update-trace.log in engine sibling.

Fix snapshot970d78681594de354987a027b65142c409072379 adds checked Tid+Header
claims compared under the exclusive predecessor latch before heap mutation.
Changed claims become serialization failures. All production transactional
update/delete/retag/catalog-delete and rollback paths use checked targets;
raw low-level heap calls require exclusive ownership. Existing Stage/WAL
atomicity is preserved. docs/design/mvcc.md records the subsystem contract;
no normative or model ruling changed.

Five focused tests passed before final helper relocation. Tests include native
paired increments/reopen, deterministic actual overlap, stale same/cross-page
claims and recovery/rollback protection. Sequential stale-target tests usually
reject early; a separate late cross-page interleaving is not forced. Independent
review of the exact final snapshot found no actionable blocker, including the
static Stage proof for that path. Report: work/briefs/review-update-ownership.md.
Reviewer ran scoped house-check, not builds/tests. Branch authoritative check
is running on the final snapshot; receipt and main integration still pending.

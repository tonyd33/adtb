---
created-at: 2026-10-27
status: in-progress
kind: feature
priority: 850
summary: campaign has no repeatable deterministic load workload checking persisted transaction and secrecy invariants
branch: campaign-load
---

# Feature: Campaign simulated workload

## Report

Campaign live calls now work locally, but no deterministic application actor
workload checks concurrency, persisted invariants and player secrecy across
seeded schedules and reopen. This is the next campaign milestone because the
application primarily exposes engine defects. Any suspected engine violation
stops expansion for a minimal regression and a principled general engine fix.
No app locks, swallowed errors or alternate queries to mask a defect.

## 2026-10-28: authorized bounded track

Workspace campaign-load at /home/tony/code/adtb-campaign-load, base f305ffaf.
Own apps/campaign tests/load and documentation plus necessary local path dev
edges to existing adtb-sim and adtb-testkit (no new external crate family).
One owner; engine role-authority uses unchanged embedded API and lands its
provider before future transport consumers. Local brief brief-campaign-load.md.

Exercise registered campaign1.6 via deterministic SimRuntime/open_in over two
isolated campaigns, small default seed count and established profile::sweep
mechanism for deep runs. Capture attempts/committed outcomes and reproducible
seed/operation history. Check conflicting next_turn advancement, unique plan
numbers, competing starts, duplicate reveal idempotence, shared/secret text and
link agreement, known-only player views and persistence after reopen. Force or
measure actual simulated overlap; distinguish unforced cases. No full200campaign
population, crash injection or p99 latency claim from the bounded first slice.

Acceptance: affected campaign tests, strict clippy/fmt/house and authoritative
branch check, lead independent review and post-land integration check. Command
from worker root: direnv exec /home/tony/code/adtb env DEVENV_ROOT="$PWD"
CARGO_TARGET_DIR="$PWD/target" check. Retain exact tested/base ids, stable-input
checks, final exits, logs, timing and unfinished scopes in a verification receipt.
Any suspected engine issue takes priority over this and feature expansion.

## 2026-10-28: preserved pending general engine sequence repair

Paused snapshot79f120a6f21fc342fe7b08df84113228d21641ed on f305ffaf,
/home/tony/code/adtb-campaign-load. Engine reservation issue discovered by
role-authority audit preempts campaign. No workload-reproduced engine defect.
Single seed4 focused pass7.078s; already-running deep8seeds collected0,63.736s,
with each seed replayed twice and identical typed ledger/trace/database bytes.
These passes are scoped evidence only, not final branch/full verification.
Initial compile and incorrect refusal-oracle failures retained in target.

No active campaign processes. Worker ended with handoff-campaign-load.md and
receipt-campaign-load.md. Fresh successor after engine repair: rebase, change
numbers.sort to sort_unstable, rerun final default/deep affected checks and strict
lint/fmt/house, then full branch check, independent review and main integration.
Default is a measured small sweep exception to2s; no scale/crash/p99 claim.

## 2026-10-28: resume after verified engine repair

General repair96744632 passed independent review and full branch/main checks
2357root/46campaign/allstaticdocs. Fresh successor resumes preserved campaign-load
snapshot79f120a6, rebases onto currentmain and finishes existing bounded workload
scope. No feature expansion. Apply trivial sort_unstable correction, inspect
all persisted/committed winner and privacy assertions, measure exact default
runtime and final deterministic replay/deep evidence, then fullcheck/review/main
integration. No implementation workaround. Fresh brief-campaign-load-finish.md
points to prior handoff/receipt; durable acceptance remains this ticket.

## 2026-10-28: independent oracle findings, verification interrupted

Fresh review of f02b917b on06676b27 found two meaningful oracle gaps, no observed
engine defect: Source was always permitted in playerknown results before reveal,
and party_encounter only searched secret sentinels without checking campaign2
has no encounter. Correct explicit phaseknown expectations (Source absent before
its revealphase, eitherduring, presentafter; othercampaign phasefixed), and exact
encounter identity/cardinality/roster plus emptyotherparty. Keep committedwinner,
seededretry and replay checks. No app/engine/query workaround.

Worker verified isolated processgroup then interrupted obsolete fullcheckSIGTERM143;
source/hashes stablef02b at interruption. Rootclippy passed22.42s; root tests,
campaign/static stages unfinished; NO overall pass. Retain target/load-final-full-
before-privacy-review/. Prior focused5commands0 onf4abc5e6,166.413s, default10.874s
and deep8replayseeds64.845s are scoped evidence only. Final correctedsource must
pass affected/default/deep/lint/fmt/house, freshdelta review and authoritative
fullcheck/mainintegration. Rebase onto currentmain before finalcheck.

## 2026-10-28: final continuation after catalog guard

Fresh successor in campaign-load resumes459ee9ff and rebases onto main15004a7f.
General guardd2716fb0 is independently reviewed and combined-main fullcheck0
(2366root/46campaign/allstaticdocs), so the engine-first pause is satisfied.
Ownership remains nine app paths; preserve corrected privacy/encounter oracles.
Refresh default/deep replay, timing prose, lint/fmt/house and authoritative branch
check on stable inputs, fresh exact review then land/maincheck. No new features
or engine workaround. First cold/full build slot belongs to campaign; role
provider waits and lands second after rebase. Brief-campaign-load-resume.md;
prior failures/interrupted143 are retained and not recast as full verification.

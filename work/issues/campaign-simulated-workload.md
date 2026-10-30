---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 850
summary: campaign has no repeatable deterministic load workload checking persisted transaction and secrecy invariants
closed: 2026-10-28
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

Workspace campaign-load at ../adtb-campaign-load, base f305ffaf.
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
from worker root: direnv exec ../adtb env DEVENV_ROOT="$PWD"
CARGO_TARGET_DIR="$PWD/target" check. Retain exact tested/base ids, stable-input
checks, final exits, logs, timing and unfinished scopes in a verification receipt.
Any suspected engine issue takes priority over this and feature expansion.

## 2026-10-28: preserved pending general engine sequence repair

Paused snapshot79f120a6f21fc342fe7b08df84113228d21641ed on f305ffaf,
../adtb-campaign-load. Engine reservation issue discovered by
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

## 2026-10-28: final policy integration and preserved failed check

Rebased app sourcec16c962e/base15004a7f received fresh independent full9path
architecture/correctness clearance. Focused default11.050s/deep8seeds64.946s
(two replays/seed), strict appclippy/fmt/house all0 with stable inputs/revision.
Final prose4b02d72c updates only these timing values; executable app delta empty.
First authoritative fullcheck4b02d72c collected exit2 after2366root/8skipped and
47campaign/1skipped passed. Campaign cargo-deny refused the approved local dev
edges absent from shared ROOT deny.toml; root dependency policy passed; later
docs/size/house unfinished. Failed evidence target/load-resume-full-before-policy.

Campaign successor additionally owns exactly3allow entries in root deny.toml:
adtb-sim, adtb-testkit and transitive adtb-reference. No new external package or
version, no role-track overlap. Both targeted root/campaign cargo-deny passed0
on fde15497/base15004a7f,stable inputs. Final fullcheck now running on that frozen
state, not yet passed. Lead will review exact policy delta and combined maincheck.
The first reported app-local deny path was corrected: no such policy file exists.

## 2026-10-28: resolved

Landed e4d7aa6c, test(campaign): add deterministic collision workload. Final
source fde154972241a16ea58daa4e15df1e929de89eb8 on15004a7f passed fresh
architecture/correctness review; lead separately reviewed measured timing prose
and exactly three approved local dependency allow entries. The post-rebase delta
is only two ticket metadata files. No app production, bundle or migration edit.

Final branch full check and house passed0 on stable inputs/revision,90.538s,
14:56:42Z–14:58:12Z. Engine2366/8skipped, campaign47/1skipped; all lint, dependency,
size and native docs checks passed. Focused default seed4 replayed twice11.050s;
deep seeds4..11 replayed twice64.946s, typed ledger/trace/file bytes identical.
New test took14.264s in the full campaign suite; this bounded sweep exception to
2s is documented. Initial full exit2 at campaign dependency policy is retained,
not counted as a pass; final policy correction and rerun are explicit.

Lead main full check also passed0 on clean494a53b3b076e2b84920ff2432c5d10f4a4986ac,
parent e4d7aa6c: exact command from root direnv exec ../adtb env
DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check. UTC14:59:59Z–15:02:08Z,
129s; permission/queue/execution-only unknown. Engine2366/8skipped50.079s,
campaign47/1skipped20.652s, all static checks passed. Docs10tests10.247s,
420syntax inputs/61pages/6730links/4995fragments/0failures. All tracked and local
root/app Cargo config hashes and revision match before/after. Dev shell
Rust1.98.1/Cargo1.98.0, configured defaults. Main log target/campaign-load-
integration/check.log SHA256bde608f4688bbba31ee9959848d954b08367869dd2bca2ee577e39b894c04611.

Receipts receipt-campaign-load-resume.md and receipt-campaign-load-main.md; fresh
reports review-campaign-final.md and review-quality28.md in work/briefs. Original
sources/logs/fixtures retained;964nondebug files copied and SHA256verified under
target/campaign-load-resume-evidence before completed debug cleanup. Duplicate
backup fixture trees are verified in tmp.tar.gz; original sibling tmp retained.
No engine failure or workaround observed. Full200campaign scale, kill/roster,
crash/fault injection, complete isolation history and native latency/p99 remain
uncovered; broad prealpha-app stays open.

## 2026-10-28: resolved

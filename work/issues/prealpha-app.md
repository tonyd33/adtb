---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 850
summary: no application exercises adtb end to end
branch: campaign-live
depends-on:
  - milestone-7-engine
tags:
  - prealpha
  - test
---

# Feature: A first application on the pre-alpha

## Report

An application, chosen by Tony, written against the pre-alpha API: its schema, registered queries and transactions as ad hoc text for now, and a workload for the load harness. Gaps it finds in the language, model or API become tickets or design-space questions.

## 2026-10-25: deferred

Tony: build the application once the engine is ready, not earlier. It waits for milestone 8 (embedded alpha); the interpreter-backed pre-alpha tickets are closed.

## 2026-10-25: gate

Tony: the goal is a real application on adtb, stress tested under
simulated load. The work starts when milestone 7 exits: the engine
passes the single-version suite with the differential and partition
tests (`milestone-7-engine.md`). Milestone 8's evolution is then proven
on this application: its first schema changes are the first real
migrations. There is no interpreter-backed pre-alpha.

## 2026-10-26: chosen

Tony left the choice to the lead. The first application is Partcrate (working name, trademark and domain unchecked), a local-first desktop app for electronics parts inventory and small-batch builds, sold as a one-time licence. Parts, lots, builds and orders are relation-level sums; reservations reference `builds.Kitting`; aggregate denial constraints bound reserved quantities; releases 1.1 to 1.3 ship as typed migrations. The stress workload is a simulated contract assembler. The market study, scope and risks are in `work/app/choice.md`.

## 2026-10-26: changed

Tony chose the runner-up instead: a local-first tabletop RPG campaign manager, for his own use first, with no revenue target. Its code and docs live in this repo under `apps/campaign/` until it is finished, then move to a repo of their own. Partcrate stays in `work/app/choice.md` as the studied alternative.

## 2026-10-26: shape

Tony: the GM edits and players get a read-only view of what their party has discovered. System-agnostic, with no stat blocks or rules. It runs as a local web server opened in a browser, so phones and tablets on the LAN can join. Version 1 covers session prep (NPCs, locations, factions, quests, linked notes), running a session live (initiative, encounter tracker, fast lookup) and campaign history (session logs, timeline, what the party knows, quest outcomes). Maps come later.

## 2026-10-27: engine-first priority

Tony: finish `api-doc-register`, then prioritize campaign work ahead of
`version-translation`. Campaign is primarily a tool for finding engine bugs.
Any engine issue exposed during campaign work takes full priority over the
application. Fix it on principle in the engine, rather than adding an
application workaround merely to unblock campaign development.

Suspend application feature expansion when an engine issue appears. Record
the reproduction and relevant contract, dispatch an engine track with a
regression, review and verify the general fix, then resume campaign work.
Design gaps still go through proposed quandary points and user confirmation.

Next application track: a runnable local server using the current typed bundle,
with create campaign, add NPC, player join and reveal as one browser flow.
Validate player-secret filtering, concurrent requests and persistence across
restart. Read `apps/campaign/docs/design.md` sections 2, 4, 8 and 9, and the
current `apps/campaign/docs/gaps.md`; report engine gaps before adapting the app.

## 2026-10-27: first browser flow landed

Landed `dd126af5`: runnable GM login/create campaign/NPC/text edit, player seat
join, reveal and shared-only player pages. Current `campaign-1.4` bundle pins
unchanged schema version 4; original bundle bytes preserved. A real-file test
opens a database registered with the original 1.3 bundle in the new server.

Workspace and lead post-land full checks passed: 2,329 root tests (8 skipped),
31 campaign tests (1 skipped), clippy, formatting, root and campaign dependency
policy, source-size and house checks. TCP and Chromium desktop/mobile flow
passed, including secret filtering, CSRF/authorization, escaping, restart and
seat revocation. Lead log: target/campaign-first-check.log. Handoff:
work/briefs/handoff-campaign-first.md. Real-file HTTP tests exceed the usual
2-second test target because migration/reopen setup dominates (roughly 4–10s).

No engine defect was reproduced. Initial same-name bundle modification was
caught in review and corrected as an application release error; the engine's
refusal was correct. First slice remains deliberately small: manual refresh,
plain text and no full prep/live/history UI or simulated application workload.
The broader ticket remains in progress with the engine-first policy above.

## 2026-10-27: prep slice awaiting review

Work is preserved in jj workspace campaign-prep at
/home/tony/code/adtb-campaign-prep. Brief and live checkpoint are
work/briefs/brief-campaign-prep.md and handoff-campaign-prep.md in the main
checkout. User requests a shutdown handoff; finish existing validation only,
then resume review and landing in a later session. No new feature tracks.

The slice adds prep entity forms, atomic name-resolved shared/secret links,
backlinks and filtered player rendering. Additive migration 0005 enforces
the existing application's same-campaign rule for NPC/Fallen locations and
Item holders; these constraints were missing from the app schema. This is
not an engine defect. New immutable campaign-1.5 pins schema 5; historical
bundle tests retain their actual schema 4. Version translation stays deferred.

Six focused real-file prep tests pass, including concurrent text/link
agreement, more than 20 names, Unicode search, valid old-file upgrade and
refusal of invalid old-file data. Final checks/browser evidence and review
are still pending at this checkpoint. Read the updated handoff before resuming.

## 2026-10-27: shutdown validation result

The six prep integration tests and desktop/mobile Chromium walkthrough passed;
the temporary browser server exited cleanly. House-check passed. Full check
was interrupted with exit 143 (SIGTERM) during cold root test compilation,
before root tests ran; root clippy had passed. The cause is unexplained and
the check was not restarted for shutdown. This is not a full-check pass.

Preserve campaign-prep/target/check.log, prep-migration.log, prep-browser.log
and browser screenshots in the sibling workspace. No campaign code landed.
Next session must review the diff, finish full verification, then land only
verified work and run main's post-land check. Main local docs server stopped.

## 2026-10-28: prep resumption scope and gates

Continue only campaign-prep in `/home/tony/code/adtb-campaign-prep`.
Original source snapshot `1c0b7a1f` was preserved and rebased without conflicts
onto main `652b3d28388bfb2d0e52dbe4e336f1dd97fb91f4`; no implementation
changes were made during reconciliation. No active workers/checks were found;
64 GB was free. Preserve all old target logs, browser artifacts and source.

Scope: prep forms for Region, Settlement, Site, Faction, Party, Item and NPC;
applicable references; exhaustive name lookup and Unicode search; shared/secret
`[[Name]]` and `[[Name|label]]` links saved atomically in the retried registered
transaction; mentions/backlinks and known-only player rendering. Additive schema
5 supplies missing application same-campaign constraints for NPC/Fallen
locations and Item holders. New campaign-1.5 pins schema 5; all earlier bundle
and migration bytes remain immutable. No new dependencies, engine changes,
version translation, live sessions or other feature expansion.

Acceptance requires persisted-row and HTTP coverage of kinds/references across
reopen, more than 20 names, Unicode search, atomic shared/secret replacement and
removal, duplicate mentions, concurrent text/link agreement, aliases, malformed
syntax, escaping, unknown names, cross-campaign refusal, player visibility and
backlinks, valid old-file upgrade and refusal of invalid old-file rows. Review
bounded desktop/mobile evidence against unchanged inputs or refresh it.

Next: one fresh verification owner and independent code review of the final
branch. Worker must establish actual sibling and shared-jj access before work;
permission failure blocks the worker. From the campaign workspace root run:

```sh
direnv exec /home/tony/code/adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check
work/tools/house-check
```

Retain a new status-preserving log and a receipt with full tested/base ids,
input stability, exact scope, final exit status, incomplete stages and timing.
The full check includes final campaign formatting, clippy and tests, not only
focused prep tests. The earlier exit 143 remains unresolved until diagnosed;
a new successful run does not establish its cause. Suspected engine defects
stop application work for a reproduction/report, never an app workaround.

Landing depends on independent review and successful final branch checks;
propose the commit subject first. The lead owns the post-land main integration
check and dated result. No push, artifact cleanup or unrelated tracks. The
broader application ticket remains open after this bounded prep slice.

## 2026-10-28: prep slice landed and integrated

Landed `4c4b522f061850d0a8aaf151850ea3a780f0278e`,
`feat: add campaign prep and atomic entity links`, after independent fresh Pi
review found no actionable correctness/security findings or suspected engine
defects. All earlier bundle and migration bytes remain unchanged. No app
workaround, engine change, new dependency or version translation was introduced.

Branch verification tested `d6a0b7994dd3f1ed53454fd4893ddebe2f3b5ea5`
on base `652b3d28388bfb2d0e52dbe4e336f1dd97fb91f4`; the landed tree is
identical to that tested tree. The full authoritative check exited 0 in 264 s:
2,329 root tests passed (8 skipped), 40 campaign tests passed (1 skipped), root
and campaign clippy/format/lint parity, dependency policy, size and documentation
checks passed. House-check passed. Inputs were unchanged at check boundaries.
Receipt: preserved sibling `target/receipt-pi-resume.md`; log:
`target/check-pi-resume.log`; original interrupted suite archived as
`target/suite-before-pi-resume.txt`. The earlier exit 143 remains unexplained;
this successful run does not diagnose its cause.

The lead then ran the same authoritative command from main on the landed
commit (empty working copy `1140b00202f9df4b0bd253416f66b5ea0220a616`):

```sh
direnv exec /home/tony/code/adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check
```

Main integration exited 0 in 124 s, from 2026-10-28T01:58:19Z to
02:00:23Z. All 2,329 root and 40 campaign tests passed, with the same skips;
all lint, formatting, dependency, size and documentation stages passed.
Documentation checked 61 pages, 6,710 links and 4,975 fragments with no failures.
Tracked inputs and local build configuration were unchanged at check boundaries.
Main tix check and scoped house-check passed. Evidence is in main
`target/campaign-prep-orchestration/main-check.{log,status,timing}` and
`main-inputs-{before,after}.sha256`; root suite remains `target/suite.txt`.
Permission wait was not observed; execution-only and queue timing are unknown.

Historical desktop/mobile browser evidence was inspected and reused explicitly
on unchanged app, engine and build inputs since source snapshot `1c0b7a1f`;
it was not rerun. Script/log/screenshots remain in the preserved campaign-prep
`target/`. Independent review noted that concurrent HTTP tests check persisted
agreement without forcing a retry, and fresh invalid-reference tests are
narrower than old-file refusal coverage. No implementation defect was established.

The landing tool forgot the campaign-prep jj registration, but its directory,
source and artifacts were retained. Prior test fixtures were archived before
reruns; no campaign-prep cleanup or push occurred. Main's prior verification
artifacts were also archived before integration. The review-only workspace and
its report are retained. Branch tix/quandary validation encountered pre-existing
missing local-only records; current main tix and validated rulings were used.
Those separate resumption limitations are not claimed resolved.

This prep slice is complete. The broader application and simulated workload
remain unfinished; leave this ticket open and unassigned pending user direction.
No further application, engine, docs-watch or SkillOpt track was started.

## 2026-10-28: live-session slice authorized

User asked Codex lead to resume and proceed. Workspace campaign-live at
/home/tony/code/adtb-campaign-live starts from d191d0f1. One implementation
agent owns apps/campaign; no overlapping writer. Local detailed brief:
work/briefs/brief-campaign-live.md. Existing standing commit/land permission
applies; no push or hosting work.

Scope: plan/list sessions by party, edit prep, start a session, prepare an
encounter and cast/extras, enter initiative rolls, advance turns/rounds, edit
standing/tracker, end encounters/sessions and read played-session detail.
Player Now/recaps use player-only typed queries; manual refresh is sufficient.
No SSE, quest/timeline UI, midfight join/leave, kill UI or version translation.
Initial initiative sorts descending roll with stable creation-order ties.
Preserve prior bundles/migrations; new callable changes use campaign-1.6 at
schema 5. Any application migration requires a concrete reported model gap.

Acceptance includes real-file HTTP lifecycle/reopen and prior1.5-file upgrade,
persisted turn/round checks, concurrent turns advancing exactly twice,
concurrent plans with distinct numbers, competing starts leaving one live
session, secret/prep/log/tracker filtering, authorization/CSRF/revocation and
cross-campaign/party/session/encounter input refusal. State whether concurrent
tests force or merely permit overlap. Any engine issue stops app expansion
for a principled engine repro/fix, never an application workaround.

From workspace root, run focused campaign tests while iterating, then:

```sh
direnv exec /home/tony/code/adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check
work/tools/house-check
```

Retain status-preserving logs and verification receipt with full tested/base
ids, unchanged-input checks and incomplete stages. Lead owns independent
review, verified landing and main post-land full check. Keep evidence and
unfinished work in handoff-campaign-live.md and this tracked ticket.

## 2026-10-28: live slice paused for engine defect

Preserve campaign-live snapshot9135c16e and handoff-campaign-live.md. Full
campaign run failed44pass/1fail/1skip; direct unchanged1.5 counter/turn calls
confirmed concurrent-update-lost-advancement. Fresh engine-update-loss agent
owns engine-layer regression/fix. No application workarounds or landing.
Wire track also paused. Engine correctness has full priority.

Live browser passed, but full check was not run and current source is not
verified for landing. Outstanding application review: lifecycle pre-read vs
write races and strict date digit validation. After verified general engine
fix lands, dispatch fresh application successor using preserved handoff, fix
those review points, rerun full campaign/root checks and independent review.

## 2026-10-28: live-session work resumed after engine repair

General engine lost-update repair ef7361f9 is landed, independently reviewed and
full branch/main check verified2334root/40campaign/allstaticdocs. Fresh successor
continues preserved campaign-live workspace, rebased onto eb2f61637b74. Source
snapshot e899dfe4 retains previous implementation and both failing witnesses.
Scope/acceptance remain the prior live-session section. Resolve atomic lifecycle
application rules and strict date-digit parsing; preserve failing-run evidence,
rerun direct/live witnesses, browser and final full check. No engine workarounds.
Any new engine issue stops expansion and restores general engine fix priority.
Lead owns independent review, landing and main integration.

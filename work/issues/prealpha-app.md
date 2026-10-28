---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 850
summary: no application exercises adtb end to end
branch: campaign-first
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

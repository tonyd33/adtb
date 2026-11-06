---
created-at: 2026-11-05
status: resolved
closed: 2026-11-05
kind: feature
priority: 285
summary: slot refuses a brief that fixes a recurring defect class site by site while its mechanism ticket is open
---

# Feature: A dispatch gate for recurring defect classes

## Request

## Proposal

## Open questions

## 2026-11-05

From the maintainer asking how to stop site-by-site fixes in a class without a reminder. Brief work/briefs/brief-class-gate-r1.md, slot-43. Every bug ticket carries a class tag or class-none; slot NAME BRIEF refuses a site fix in a class with three tickets and an open mechanism ticket unless the brief says why it cannot wait.

## 2026-11-05 resolved

Landed as `feat: refuse a slot whose brief fixes a recurring class site by site`. `slot NAME BRIEF` reads one `Ticket: STEM` line (two refuse; fenced lines ignored) and refuses a bug ticket with no class tag (or class-none with a class), and a site fix with no `Site fix because:` reason in a class of three or more instances (wontfix and mechanism tickets not counted) while any of its mechanism tickets, a comma-separated set in defect-classes.md, is open or in progress. A missing table, no class rows, an empty mechanism cell or an unknown stem refuse every brief. Review found a second Ticket line bypassing the gate; fixed. 24 removals each fail slot-test. 111 open bug tickets are untagged and get tagged at dispatch.

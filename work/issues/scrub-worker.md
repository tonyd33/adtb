---
created-at: 2026-10-29
status: resolved
kind: feature
priority: 415
summary: "no background scrub: ScrubInterval is unread and the engine has no background worker"
closed: 2026-10-30
---

# Feature: Scrub worker

## Request

A background thread reads every page of the file over each `ScrubInterval`, sets marks by the rules of catalog 11.2 and writes findings under the `scrub` scope. The cursor persists across short processes. Acceptance: simulated rot found within an interval with no call touching the page, finding row and mark present.

## Proposal

## Open questions

## 2026-10-30

Landed on main. Check passed, 3287 tests. Follow-ups: scrub-follow-ups.

## 2026-10-30: resolved

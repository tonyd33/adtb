---
created-at: 2026-10-29
status: in-progress
branch: scrub-worker
kind: feature
priority: 415
summary: "no background scrub: ScrubInterval is unread and the engine has no background worker"
---

# Feature: Scrub worker

## Request

A background thread reads every page of the file over each `ScrubInterval`, sets marks by the rules of catalog 11.2 and writes findings under the `scrub` scope. The cursor persists across short processes. Acceptance: simulated rot found within an interval with no call touching the page, finding row and mark present.

## Proposal

## Open questions

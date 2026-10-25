---
created-at: 2026-10-25
status: open
kind: feature
priority: 810
summary: "no write states: readers-first, pending drops and staged jobs do not exist"
depends-on:
  - operator-log
tags:
  - engine
---

# Feature: write states, readers first and staged jobs

## Report

Implement `docs/spec/evolution.md` §6 and §8 and
`docs/design/schema-change.md` §2: write states, readers-first, pending
drops, staged jobs.

Prerequisite: `operator-log.md`.

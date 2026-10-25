---
created-at: 2026-10-25
status: open
kind: feature
priority: 820
summary: "no bundles: families, registration checks, pinning, translation, leases and supersession do not exist"
depends-on:
  - operator-log
  - write-states
  - registered-calls
tags:
  - engine
---

# Feature: bundles and families

## Report

Implement `docs/spec/applications.md` §3 to §6, §8 and §9 and
`docs/spec/evolution.md` §7: bundles and families, registration
checks, pinning, translation of pinned queries, leases, supersession
at open, `hide new constructors`, the recheck of registered queries on
a schema change. Settle `schema-change.md:75`, the recheck contract.

Prerequisites: `operator-log.md`, `write-states.md`,
`registered-calls.md`.

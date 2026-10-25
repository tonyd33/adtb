---
created-at: 2026-10-25
status: open
kind: feature
priority: 830
summary: "an embedded database cannot be opened with its bundles, and host functions and bulk append do not exist"
depends-on:
  - bundles
  - in-process-api
tags:
  - engine
---

# Feature: embedded open steps, host functions and bulk append

## Report

Implement `docs/spec/embedding.md` §2 to §4, §7 to §10 and
`docs/spec/interop.md` §9: the embedded open steps with bundles,
calls, cancellation, configuration, bulk append, host functions.

Prerequisites: `bundles.md`, `in-process-api.md`.

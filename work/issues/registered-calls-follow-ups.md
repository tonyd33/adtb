---
created-at: 2026-10-27
status: open
kind: feature
priority: 835
summary: "registered call arguments are not checked against their parameter types, and limits, feeds and bundle aggregates refuse registration"
depends-on:
  - registered-argument-values
tags:
  - engine
---

# Feature: registered call follow-ups

## Report

- `Registered::call` checks the number of arguments but not their values against the parameter types.
- A declared `limit`, a feed, or a bundle aggregate refuses registration.
- Two tests in `crates/adtb-sim/tests/registered.rs` take 3.9 s and 2.2 s, over the 2 second budget, and the binary adds about 4 s of wall time to the default run.

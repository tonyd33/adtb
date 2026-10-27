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

## 2026-10-27: registrations landed

Registered calls landed without persistence: registrations live in `OpenOptions::registrations` for one open and are not written to `catalog::bundles` or `catalog::registered`, and a registration is checked at the version open leaves with no pin or translation. `Registration::new(name, text)` takes a bundle file without its header; wrap it with the header here. `docs/design/server.md` section 4 has a TODO for how a pin, translation and lease reach a registration. Start from `checked` in `crates/adtb/src/api/registered.rs`.

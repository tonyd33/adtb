---
created-at: 2026-10-27
status: open
kind: feature
priority: 822
summary: "a bundle pinned to an earlier version cannot run: reads are not mapped back through the operators' inverses and writes are not mapped forward"
depends-on:
  - bundles
tags:
  - engine
---

# Feature: serve a pinned version by translation

## Report

`evolution.md` section 7: a bundle pinned to version `v` reads the current data mapped back to `v` through the inverses of the operators since `v`, writes through the operators forward, and encodes results at `v`. Section 7.2 decides translation at apply, 7.3 states expired bundles, and 7.4 the fields a translated write lacks. The `bundles` track pins and checks bundles but serves only bundles pinned at the current version.

Ruling for `spec-registered-explain-pin`: a registered call's parameters are typed at the bundle's pinned version, and its argument values map forward to the current version as written values do; its results are encoded at the pinned version.

---
created-at: 2026-10-25
status: open
kind: feature
priority: 335
summary: "the simulator never switches threads between an optimistic read and its validation, or before an exclusive latch"
tags:
  - sim
---

# Feature: Switch points inside optimistic reads

## Report

The simulator switches threads only at I/O and at contended waits.
So in the B-link tree nothing switches between a writer's last
optimistic read and its exclusive latch, or between an optimistic copy
and its validation. Turning off move-right inside `latch` passes every
test. Expected: under the simulator, `adtb_sys::pages` takes a seeded
switch point inside optimistic reads, between the read and validation,
and before taking a latch, so those races and failed validations are
exercised.

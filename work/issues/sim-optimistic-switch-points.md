---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 335
summary: "the simulator never switches threads between an optimistic read and its validation, or before an exclusive latch"
closed: 2026-10-25
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

## 2026-10-25: resolved

`adtb_sys::pages::switch_points` lets the simulator switch threads at
the start of an optimistic read, inside its copy, before validate and
upgrade, and before every latch wait, at `Config::switch_rate`. At
0.01 the B-link thread tests catch the removal of move-right, the
low-key check in `claim`, and retain judging from a stale copy. The
unlinked-page check in the descent is kept: without it a descent can
follow a freed and reused right link and report `Malformed`. Detection
of the low-key check is thin:
[`btree-low-key-test.md`](btree-low-key-test.md).

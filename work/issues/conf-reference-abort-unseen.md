---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 90
summary: "work/tools/conf reports no difference when the reference run panics, and spins forever on a one-CPU machine"
closed: 2026-10-27
tags:
  - quality
  - tooling
  - devex
---

# conf passes a file whose reference run panics

## Report

`on_reference` at `work/tools/conf:56-58` pipes only the reference runner's stdout through `sed` and ignores its exit status. A panic writes only to stderr, so `differs` stays 0 and `conf` exits 0. The engine side checks an `adtb-conformance-done` marker for this case; the reference side has nothing.

In the same file, `workers=$(($(nproc) / 2))` is 0 when `nproc` is 1, and the throttle loop at `:94` never admits a job.

Fix: capture `2>&1` and the status, report `FILE:0: reference: aborts (N): ...` as the engine side does, and floor `workers` at 1.

## 2026-10-27: done

`on_reference` in `work/tools/conf` keeps the runner's stdout and stderr apart, and reports `FILE:0: reference: aborts (N): ...` (the last three lines) when the runner exits above 1, or exits 1 having printed no difference. `workers` is floored at 1. Checked by hand with a stand-in runner that exits 134 and `workers` forced to 0: both files report the abort and the run exits 1. Nothing stays open.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qtools track.

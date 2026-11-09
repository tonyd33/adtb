---
created-at: 2026-11-09
status: open
kind: bug
priority: 320
summary: "probe_unique calls heap::refuse_named while holding the index's unique-check lock, so it records damage under a lock every other refusal site releases first"
tags:
  - engine
  - class-not-atomic
---

# Bug: Unique probe records damage under the unique-check lock

## Observed

In crates/adtb/src/write/apply.rs, `probe_unique` calls `heap::refuse_named` while it holds the index's unique-check lock. Every refusal site that heap and write gained in structure-version-owner steps 7 and 8 releases its latch, write or lock before recording damage, so damage recording never waits on a page while holding another. `refuse_named` can answer `Ok` without refusing, after which the probe continues under the lock, so releasing the lock first means restarting the probe.

## Expected

The probe releases the unique-check lock before any damage is recorded, and restarts or fails cleanly when `refuse_named` refused nothing; the restart's termination is argued, not assumed.

## Reproduction

Plant a version the index names whose page no longer names it (through the testing surface), and run an insert that probes that key. Found by the step 8 review of structure-version-owner; no deadlock is shown.

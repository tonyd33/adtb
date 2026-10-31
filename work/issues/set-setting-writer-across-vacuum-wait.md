---
created-at: 2026-10-31
status: open
kind: bug
priority: 575
summary: "set setting holds bundle_writer while it waits for the vacuum lock, so grants, registrations and retires stall for a whole rebuild"
tags:
  - quality
  - engine
---

# Set setting writer across vacuum wait

## Report

crates/adtb/src/api/command.rs:321 takes `bundle_writer`, then calls `hold_vacuum`. A call that cannot suspend polls there holding the writer lock: an in-process call, and a server call's command after its first. A rebuild holds the vacuum lock for its whole run, so until it ends:

- `register`, `grant`, `revoke`, `drop_role`, `retire` and `supersede` block on `Lock::lock`, which no cancel or deadline reaches, and on a server each holds a worker;
- `Engine::renew` finds `try_lock` taken and skips renewals, so a long rebuild can run a lease toward expiry.

A suspending call also takes `bundle_writer` before it can suspend, so it can block its worker on the lock, which the vacuum lock's `Blocked` path exists to avoid.

Found by reading; not run. Repro: start `reindex ts` with a gate held, run `explain from t in ts select t` then `set setting RetentionWindow = { length: 2h }` in one text, then `grant monitor to ann` from another thread, cancelling it.

Actual: the grant waits for the rebuild and ignores the cancel. Expected: the grant is unaffected by the wait.

Direction: take the vacuum lock first and `bundle_writer` only for the write. No holder of `bundle_writer` takes the vacuum lock, so the order is free. Only `set_setting` needs the writer, for `latch_expired` and the settings change.

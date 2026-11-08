---
created-at: 2026-11-08
status: open
kind: bug
priority: 700
summary: "a Pinned log hold dropped without its LogPin holds Log::needed for the rest of the open, and #[must_use] does not fire once it travels in a tuple or Option"
tags:
  - quality
  - engine
---

# A dropped log pin stops log recycling for the rest of the open, unchecked

## Report

`Pinned` (crates/adtb/src/storage/log.rs:553) has no `Drop`; the log count it stands for is given back only when a `LogPin` built from it drops. `#[must_use]` on the struct does not fire once the value travels inside `Checkpointed` (crates/adtb/src/storage/pager.rs:178) or the `Option<Pinned>` of `checkpoint_pinning` (crates/adtb/src/storage/checkpoint.rs:94) and is bound to `_`, as `log_checkpoint` does at checkpoint.rs:79. That site is sound only because it passes `pin = false`. A caller that passes `true` and drops the pin keeps `Log::needed` at its LSN for the rest of the open, so log recycling stops with no error. The storage step 5 record states "`Pinned` that `LogPin` consumes" as enforced; nothing enforces it.

Fix direction: have `Append::checkpoint` hand back the `LogPin` itself, so no `Pinned` exists outside `Log::pin` and `LogPin::new`, or give `Pinned` a `Drop` that panics unless consumed. A test that a pinning checkpoint's pin, dropped, lets the next recycle remove its segment.

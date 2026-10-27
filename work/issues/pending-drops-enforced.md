---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "a drop held pending by a bundle reader is not enforced by the write path, and pending drops do not carry across opens"
tags:
  - engine
---

# Bug: pending drops are not enforced

## Report

The catalog holds a drop pending while a live bundle reads it (`evolution.md` section 6.4), and keeps the enforced program in `Catalog::checking()`, but an open builds its write path from the version program, so the dropped constraint is not enforced while it is pending. Every `Catalog::open` also starts with no readers, so a drop applied in one open has fully taken effect by the next instead of staying pending while its holders are live. Build the write path from the enforced program, and persist pending drops and their holders so they carry across opens. The ignored test `a_drop_stays_pending_while_a_bundle_live_before_the_open_reads_it` in `crates/adtb-sim/tests/bundles.rs` states it.

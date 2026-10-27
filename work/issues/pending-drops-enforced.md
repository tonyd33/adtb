---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "a drop held pending by a bundle reader is not enforced by the write path, and pending drops do not carry across opens"
closed: 2026-10-27
tags:
  - engine
---

# Bug: pending drops are not enforced

## Report

The catalog holds a drop pending while a live bundle reads it (`evolution.md` section 6.4), and keeps the enforced program in `Catalog::checking()`, but an open builds its write path from the version program, so the dropped constraint is not enforced while it is pending. Every `Catalog::open` also starts with no readers, so a drop applied in one open has fully taken effect by the next instead of staying pending while its holders are live. Build the write path from the enforced program, and persist pending drops and their holders so they carry across opens. The ignored test `a_drop_stays_pending_while_a_bundle_live_before_the_open_reads_it` in `crates/adtb-sim/tests/bundles.rs` states it.

## 2026-10-27: done

An open binds its write path, and the engine's schema and program, from `Catalog::checking()`, so a pending-dropped constraint or check stays enforced. Pending drops and their holders are stored as roots rows (constructors 6 and 7 of the roots heap, one per object and holder), written in the migration that makes a drop pending and in each release. `Catalog::open` reads them and refolds the log from the earliest pending drop with the held objects kept. `Catalog::settle` releases every holder that is not a live reader; the embedded open calls it after registering the bundles it ships that were live. `Catalog::release` now takes the pager and transactions and is durable. `schema-change.md` §2.1 states it. The ignored bundles test passes; new tests in `bundles.rs` and `write_states.rs` cover carrying across opens, settling and durable release.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qdrops track.

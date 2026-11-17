---
created-at: 2026-11-17
status: open
kind: bug
priority: 600
summary: "a migration script is checked as Adhoc, so its rows count as snapshot rows and a stored reference's deref is total over a state the earlier operators left unchecked"
tags:
  - lang
  - class-none
---

# Migration script reads as snapshot

## Report

Found by the review of constraint-deref on 2026-11-17. A migration script is checked under `Context::Adhoc` (crates/adtb-lang/src/check/mod.rs), so its rows are snapshot rows and the deref of a stored reference is total. Per evolution.md a script reads the state the earlier operators leave, which is not yet checked, so the deref can name no row. Latent: the engine refuses scripts today (crates/adtb/src/catalog/migration.rs).

Expected: when scripts run, a script's root makes every deref partial, as constraint and row-check roots do.

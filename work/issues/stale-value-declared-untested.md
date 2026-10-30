---
created-at: 2026-10-29
status: open
kind: bug
priority: 897
summary: "no test reaches stale's Declared filter for a value-level constructor, so refusing a bundle over a constructor only declared would pass the suite"
tags:
  - quality
  - engine
  - test
---

# stale's Declared filter on value constructors has no test

## Report

`stale` skips a value-level constructor that is only declared (`now.value_stage(def, c) != Stage::Declared`, `crates/adtb/src/catalog/readers.rs:739`). 20dfc502 rewrote that line and a3e00f73 added a test named for stale finding exactly the readers that refuse an enable, but `blockers` (`crates/adtb-sim/tests/write_states.rs:1625`) calls `stale` only after the enable has applied, so every constructor is enabled by then. In `crates/adtb-sim/tests/bundles.rs` every value-level case uses `[V1, ADD_GOLD, ENABLE_GOLD]`, and the only-declared case (`bundles.rs:615`) is at relation level through `ADD_APPLE`. Found by reading; the mutation was not run.

Repro: delete the `value_stage` test at `readers.rs:739`. By reading, no test fails, and a bundle reading `users.tier` pinned before `add constructor Tier.Gold` is then refused at open as stale though Gold is not enabled.

Fix: add the value-level twin of `a_bundle_pinned_before_a_constructor_that_is_only_declared_is_not_stale` with log `[V1, ADD_GOLD]`, and have `blockers` also check `stale` for each reader before the enable, expecting `None`.

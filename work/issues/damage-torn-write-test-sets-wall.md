---
created-at: 2026-11-13
status: open
kind: feature
priority: 150
summary: "one damage search test takes 178 s and sets the wall of every damage profile run; the next slowest takes 101 s"
tags:
  - tooling
---

# Feature: Split the torn-write damage test that sets the damage profile's wall

## Request

`damage_search::a_torn_write_is_restored_from_its_image` (crates/adtb-sim/tests/storage/damage_search/mod.rs:942) takes about 178 s; the next slowest test of `profile.damage` takes 101 s. The profile ran 289 times in the week to 2026-11-13 at 175 to 220 s each, so this one test sets the wall of every run (devex timing review of 2026-11-13).

## Proposal

Split its cases across two or more tests so nextest runs them in parallel, or read a smaller case count in the default damage profile and the full count under `deep`, from the one place the suite states it. Exit: the damage profile's wall at or under the next slowest test plus build; the same cells reached under `deep` (its per-kind floors unchanged).

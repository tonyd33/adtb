---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 851
summary: "the campaign front end tests for releases 1.1 to 1.3 assert only the version number, not the folded schema"
closed: 2026-10-28
tags:
  - quality
  - test
---

# Feature: campaign front end release tests assert the folded schema

## Report

`release_1_1_adds_maps_and_items`, `release_1_2_splits_locations` and `release_1_3_adds_quest_states` (`apps/campaign/tests/front_end.rs:183-200`) assert only `version(n).version == n`, that the migration applied. Nothing checks that maps, pins and `Item` exist, that `Location` became `Region | Settlement | Site` with `Npc.location` retyped and no `Location` left, or that `Shelved` and `Abandoned` joined `Open` and `Closed`.

Fix: assert the declared constructors and sets of each folded source, as `folds_to_the_schema` does for 1.0.

## 2026-10-28: dispatched

A fresh writer owns this in workspace campaign-tests, on main cd020ae0, beside permission-successor (adtb-lang) and the other new track. Local brief: work/briefs/brief-campaign-tests.md.

## 2026-10-28: resolved

## 2026-10-28: landed

Landed as afb0b0bc (test(campaign): assert exact refusals, stored constructors and the folded schema of each release). The writer showed each new assertion fails when its property is broken. Branch check on 1e7faf8e and post-land main check on afb0b0bc both passed: root 2438 passed, 8 skipped; apps/campaign 47 passed, 1 skipped; main check 3m40s, log target/check-main-campaign-tests.log.

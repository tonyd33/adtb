---
created-at: 2026-10-26
status: open
kind: feature
priority: 851
summary: "the campaign front end tests for releases 1.1 to 1.3 assert only the version number, not the folded schema"
tags:
  - quality
  - test
---

# Feature: campaign front end release tests assert the folded schema

## Report

`release_1_1_adds_maps_and_items`, `release_1_2_splits_locations` and `release_1_3_adds_quest_states` (`apps/campaign/tests/front_end.rs:183-200`) assert only `version(n).version == n`, that the migration applied. Nothing checks that maps, pins and `Item` exist, that `Location` became `Region | Settlement | Site` with `Npc.location` retyped and no `Location` left, or that `Shelved` and `Abandoned` joined `Open` and `Closed`.

Fix: assert the declared constructors and sets of each folded source, as `folds_to_the_schema` does for 1.0.

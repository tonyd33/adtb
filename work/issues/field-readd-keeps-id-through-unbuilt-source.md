---
created-at: 2026-11-12
status: open
kind: bug
priority: 340
summary: "a field dropped and re-added in one migration keeps its id when the source between does not build"
tags:
  - class-identity-by-spelling
---

# A dropped and re-added field keeps its id through an unbuilt source

## Report

`reconcile_ids` (crates/adtb-lang/src/evolve/ids.rs:29-38) returns early while the source does not build. So `drop field T.B.z` plus `add field T.B.z ...` in one migration keeps B.z's id while `index (z) in S` still exists (the drop leaves the source unbuilt), and gives a new id once that index is gone. Consequence: `add index (z) in ts.R`, `drop index (z) in ts.R`, then that drop and re-add of B.z, then revert: applies and restores the index over the new field; the same with C.z is refused. Found in the delta review of structure-lang-identity step 5c-2.

Ruling (lead): a dropped object is gone; a re-added one is a new object with a fresh id (catalog.md §1.2), whether or not the source between builds.

Direction: assign and retire ids per operator from the operator's own effect, not from a build of the whole source. Test: both histories above give the same refusal.

Defect class: Catalog rows and identity by position or spelling.

## Reach

The engine refuses this history before the id question arises: an `add field ... fill` after a drop that leaves the source unbuilt fails with "the operator checks against the schema the operators before it produce, which does not check" (`fill_map` calls `checked()`). Only the lang `Source::migrate` API reaches it. Fixed there by structure-lang-identity step 6a-2.

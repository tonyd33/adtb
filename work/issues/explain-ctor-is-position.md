---
created-at: 2026-11-05
status: open
kind: bug
priority: 470
summary: "The plan document's ctor member holds a constructor's position in its type, not its catalog constructor id as catalog.md 7.4 states"
---

# Plan document ctor is a position, not an id

## Report

## Report

catalog.md §7.4: a routing node's `arms` and a read's `pruned` entries carry `"ctor"`, the constructor id. The engine writes the constructor's position in its type (0, 1, 2): for the users fixture, EmailPass, Google and Apple show 0, 1, 2 where their ids (§1.2, and `catalog::constructors`) are 1028, 1029, 1030. A tool joining a plan to `catalog::constructors` by `ctor` gets the wrong constructor. Identity by position, the class AGENTS.md Invariants names. Written in crates/adtb/src/plan/explain.rs (arms and pruned) and crates/adtb/src/write/explain.rs (arms).

Found in script batch 6, 2026-11-05; the explain scripts pair arms and pruned by `name` until this is fixed.

## Proposal

Write the catalog constructor id. Decide what `ctor` holds for an anonymous sum, which has no catalog id (omit the member, or state it in §7.4). Then make the explain scripts state `ctor`.

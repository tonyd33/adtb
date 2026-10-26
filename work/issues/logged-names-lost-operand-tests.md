---
created-at: 2026-10-25
status: open
kind: feature
priority: 812
summary: "Logged::names refusing an operand a later operator rewrote has no test, and a partly rewritten operand loses names silently"
tags: [quality, lang]
---

# Feature: tests for operands a later operator rewrites

## Report

`Logged::names` (crates/adtb-lang/src/evolve/names.rs:34-90) resolves an operand copied into a declaration by the source after its operator, and when that source does not build, by the copies left in the migration's result (`late`, names.rs:60). Two branches have no test in `resolved_tests.rs` or `evolve_tests.rs`:

- No piece of the copy survives (names.rs:76): the migration's names fail with "a later operator of the migration rewrites the declaration this operand was written into". A migration that adds a declaration whose source does not build until a later operator, then has a later operator rewrite that declaration whole (`alter`/`replace` of it), should reach it.
- Some pieces survive: `copy_marked` (names.rs:82) copies names only for those pieces, and the rest of the operand resolves to nothing with no diagnostic. Decide whether that is a refusal like the first case, and test it.

`an_operand_a_later_operator_edits_around_still_resolves` covers only a later edit inside a copy whose source builds after each operator.

## 2026-10-25: ruling

A partly rewritten operand never loses names silently. The text a later operator writes into it is a copy with its own marks, so its names come from that operator's source; a part with neither survives as a refusal naming the operand. Test both branches.

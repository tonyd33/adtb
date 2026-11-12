---
created-at: 2026-11-12
status: open
kind: bug
priority: 450
summary: "a merge prepends the discriminator to every unique, so a unique held jointly over the merged constructors stops holding across them"
tags:
  - class-none
---

# A merge weakens a unique over the merged constructors

## Report

`unique (n) in S` with `set S of ts = A | B`; `merge constructor T.(A | B) into M { k: K, ... }` gives `unique (k, n) in M`. Before, `n = "x"` could not be on both an A row and a B row; after, a row with `k = A` and one with `k = B` can both hold it, so the merge silently weakens a declared constraint. It also rewrites `in S` to `in M` with a new id though the clause names no replaced constructor. Cause: `merged_fields` prepends the discriminator for every unique. Pre-existing on main.

Direction: prepend the discriminator only to a clause scoped to one merged constructor; a clause over several merged constructors (or a set reaching them) keeps its fields, and a set scope keeps its text and id. Test: the scenario, then insert two M rows with the same `n` and different `k`, expecting the unique's refusal.

Defect class: none.

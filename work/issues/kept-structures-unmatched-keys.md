---
created-at: 2026-11-10
status: open
kind: bug
priority: 600
summary: "Kept::structures indexes the current heaps by a history heap's key with an unnamed panic and silently drops a history index with no key index, so free_leaked would free it"
tags:
  - quality
  - engine
  - class-none
---

# Bug: the history structure listing trusts unmatched keys

## Report

crates/adtb/src/catalog/history.rs:245 `heaps[&(r, c)]` panics without naming its invariant when a history heap has no current heap of the same key, and :251 `if let (Some(sum), Some(key)) = (sums.get_mut(r), keys.get(r))` skips a history index whose relation has no history heap or key index. That listing feeds the inventory `free_leaked` reads (catalog/mod.rs:879), so a skipped history index's pages would count as leaked and be freed. The keys agree today only because `Kept::open` and `rebuilt` build them alike; nothing checks it.

Fix direction: hold a relation's history structures in its own entry beside the current ones, so they cannot disagree, or fail naming the invariant instead of skipping.

Found in the quality review of `reviewed..main` at 7bf40103.

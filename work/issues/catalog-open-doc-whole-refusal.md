---
created-at: 2026-11-10
status: open
kind: bug
priority: 800
summary: "Catalog::open's doc promises an uncommitted schema change's pages freed, but under a whole refusal it returns before freeing them or tagging grants"
tags:
  - quality
  - engine
  - class-none
---

# Bug: Catalog::open's doc omits its whole-refusal return

## Report

`Catalog::open` (crates/adtb/src/catalog/mod.rs:699-713) says "A schema change that did not commit has every page it allocated freed". The body returns at once when `pager.refused_whole()`, leaving `changing` on the root and skipping `tag_grants`. The test `an_open_refused_whole_by_a_mark_or_its_commit_log_leaves_a_change_in_progress` pins the branch the doc does not state.

Fix direction: state the whole-refusal case in the doc as a postcondition.

Found in the quality review of `reviewed..main` at 7bf40103.

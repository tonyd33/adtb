---
created-at: 2026-10-30
status: in-progress
branch: quality-fixes33
kind: bug
priority: 562
summary: "a failed Pager::scrap leaves a blank resident page with its damage forgotten, where replace_contents restores the damage"
tags:
  - quality
  - engine
---

# Scrap failure hides damage

## Report

`Pager::scrap` at `crates/adtb/src/storage/quarantine.rs:325` calls `latch_overwriting`, which removes `pid` from `damaged`, blanks the page and adds it to `overwritten`. If `update_all_with` at line 329 then fails on an allocation bitmap page, `?` returns with the page resident, blank and reading as sound. `replace_contents` handles the same failure by calling `self.damage(pid, Damage::Checksum)`.

Later reads of that page get zeros with no error, and `overwritten` keeps a stale entry, so the next change logs an unchained image. Callers: `btree::build::retire` and `Catalog::free_leaked`. No test names `scrap`.

Found by reading; not run. Repro direction: damage a page of an old tree and its bitmap page, then `reindex`.

Fix direction: share the restore with `replace_contents`, clearing `overwritten` too, and test a failed scrap.

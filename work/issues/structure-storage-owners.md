---
created-at: 2026-11-07
status: open
kind: feature
priority: 12
summary: pager state is 37 crate-visible fields locked at 85 sites in 10 files with cross-map rules kept by hand; give each piece an owner type whose operations keep its invariants
---

# Feature: Storage state behind owners: sealed frame, dirty table, evidence, append

## Request

## Proposal

## Open questions

## 2026-11-07

Design: work/briefs/design-storage-structure-r1-report.md (local); its nine open questions go through the design space before the first step. Owners: a sealed frame (seven named unlogged writers; `disallowed-methods` on `pages::Exclusive::write` and `PageMemory::lock_exclusive`), `DirtyTable` (one map for dirty, changed, images, after_write), `Evidence` (one record per page incarnation, one judge for read-in, verify and backup), `Append` (the only holder of log and meta together), and owners for allocation, marks and the header. Subsumes page-write-escape-hatch and page-incarnation-identity's evidence authority. Steps 1 to 5 keep behaviour; 6 to 8 remove gated damage findings, each confirmed by its pinned case.

Exit: 0 crate-visible fields in pager.rs, 0 `.meta()` outside the owner module, the lock order stated and checked.

## 2026-11-07, rulings

In the design space: storage/structure/owners, free-page-evidence/verify-only, superseded-mark/skip, marks-under-held/durable.

Internal, ruled by the lead: the incarnation key is an in-memory generation per page, bumped at each chain start and `Freed`; the log and meta locks stay separate, joined only through the `Append` owner; evidence is one record per page incarnation, not maps behind one type; heap/dir_tests.rs's resident write becomes the simulator's file fault plus eviction, with a `#[cfg(test)]` named writer only if a resident copy that differs from the file is needed; `walked` moves into the evidence owner's mapping statistics; the lock-order comment is corrected in the step that seals the locks, with the marks lock as a leaf.

## 2026-11-07, step 1 landed

The frame is sealed. `PageExclusive` and `Held` have private fields, `inner` and `Shared::mem` are gone from view, and outside pager.rs a latch from `Pager::hold` only reads. Page bytes change outside a logged change only through pager.rs's private writers (page-in, image install, applied change, blank frame, overwrite, settle restore), each one item-level `expect` of `disallowed_methods`. clippy.toml lists `pages::Exclusive::write`, `PageMemory::lock_exclusive`, `PageMemory::try_lock_exclusive` and `Optimistic::upgrade`. The writes that need bookkeeping are single Pager operations (`latch_blank`, `latch_overwritten`, `latch_restored`, `replay_page_in`, `replay_image`, `replay_change`). The guard's kind replaces the `blank` flag and the `overwritten` set; `apply_freeing` takes the reservation. `blanked` stays as a set until step 5. Amended invariant: `Overwriting` carries no prior, since a failed overwrite refuses the page as `Checksum` whatever the earlier damage was, as before. One behaviour difference: a failed scrap or replace now refuses the page under the same latch, which closes a window in which a reader could take the zeroed frame as sound. Counts: pager.rs crate-visible fields 38 to 33; raw page writes and exclusive latches outside pager.rs 0. Log, header and option checks moved out of pager.rs unchanged to stay under the size limit. Next: step 2, `DirtyTable`.

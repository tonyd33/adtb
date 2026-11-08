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

## 2026-11-07, step 8 input from the step 2 review

`DirtyTable::refused` drops the page's entry at the next covering force even when the refused copy was dropped unwritten, so the frame's changes the file lacks leave the table. Existing behaviour on main. The design's `refused(pid, wrote_first)` keeps the entry in that case; step 8 decides it with `freed_durable`, with a search case of a refused unwritten page and a covering force.

## 2026-11-07, step 2 landed

`DirtyTable` in storage/pagetable.rs owns what were six Meta fields (`dirty`, `changed`, `written`, `images`, `after_write`, `wrote`) as one map whose entry carries every per-page fact, so the subset rules hold by representation. Each mutator takes a witness only its caller can make: `forcing` the held force lock (its `Covered` borrows the guard, so two forces cannot overlap), `written_out` a `Wrote` from the data write, `refused` a `Refusal` from `publish_refusal`, `replayed` and `replay_refused` a `Replaying` from recovery. `logged` and `record_kind` close in step 3 with the `Append` guard. `Meta.table` is private to pager.rs; `Shared::atomic_page_writes` moved into the table. storage.md §6 states the table and both ways an entry leaves. Counts: pager.rs crate-visible fields 33 to 26; uses of the six facts outside pager.rs 19 to 0. The search test checks the table against main's six collections, reaching a write, a change and a refusal of a covered page during a force. Next: step 3, `Append`.

## 2026-11-07, step 3 landed

`Append` in pager.rs is the only holder of the log and meta locks together: a record and its in-memory consequences (dirty entry, expected LSN, damage record, LSN map root) are made in one hold of the log, and a checkpoint's snapshot is taken in the same hold. The LSN map root lives under the log lock (`Appended { log, map }`). `DirtyTable::logged` and `record_kind` take an `Appending` witness only a live `Append` holds. The lock order is stated on `Shared` as the code takes it (recycle; page latches, with the allocation lock between a data page's latch and a bitmap page's; segments; log; meta; marks a leaf) and checked at runtime: the log and meta accessors record their holder and panic on any wrong order, including inside an `Append`, with one test per refused order. Ruling: lock order stays a runtime check, since a type-level order costs more than it buys here. Counts: pager.rs crate-visible fields 26 to 23; `.meta()` sites 54 to 39; functions holding log and meta outside `Append` 6 to 0. Next: step 4, `Evidence`.

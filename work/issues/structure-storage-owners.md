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

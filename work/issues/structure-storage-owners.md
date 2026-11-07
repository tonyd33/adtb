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

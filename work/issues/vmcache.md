---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 402
summary: "no buffer pool: no reserved virtual range, frames, eviction or optimistic reads"
closed: 2026-10-25
depends-on:
  - page-buffer-contract
tags:
  - engine
---

# Feature: vmcache buffer pool

## Report

Implement the vmcache buffer pool of `docs/design/storage.md` §10.3
over `adtb-sys` (`docs/design/sys.md` §2 and §3): reserved virtual
range, frames, the page state word, eviction, optimistic reads. Settle
`storage.md:434`, the replacement policy. Loom tests of the page state
word belong to `loom-and-miri.md`.

Prerequisite: the contract of `page-buffer-contract.md`.

## 2026-10-25: resolved

Landed as `feat(storage): share the pager across threads over a vmcache pool`: `adtb_sys::pages` (state word, three guards), a `&self` pager with clock replacement, and `commit_all`. Tested under the simulator scheduler; loom is not added.

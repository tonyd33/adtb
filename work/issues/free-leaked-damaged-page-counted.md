---
created-at: 2026-11-18
status: open
kind: bug
priority: 650
summary: "free_leaked reports a damaged or unreadable leaked page of a removed heap as counted off, so the change marker ends and its large-value heads stay overcounted"
tags:
  - quality
  - engine
---

# A damaged leaked page of a removed heap ends the change with its heads overcounted

## Report

`Catalog::free_leaked` (`crates/adtb/src/catalog/mod.rs:849-864`) reads each leaked page to learn its owner before it calls `release_page`. When that read fails with damage other than `Unreadable` it scraps the page and `continue`s; when it fails `Unreadable` the owner is `None` and `release_page` is not called. Both paths leave `counted` true, though the page may belong to a heap of `gone` whose versions were never counted off their large values.

`counted` true then lets the commit arm (`end_committed`, mod.rs:926) and the abort arm (`crates/adtb/src/catalog/migration.rs:195-198`) call `root::end_freed` and remove the change marker, so the next open does not run `lower_overcounted`. The heads those versions pointed at stay overcounted and their values are never freed.

The round-2 rule of the rewrite-leaks track: after a change marker, every head's count equals the versions that name it, and a page whose versions could not be counted off leaves the change on the root for the open to mend. `release_page` keeps it for its own read failure (`crates/adtb/src/heap/handle/decode.rs:186-199` returns false); `free_leaked`'s earlier read bypasses it.

Repro direction: in `commit_point_tests.rs`, damage a page of the heap a rewrite migration frees before `free_leaked` runs (a fault on its read), then reopen and assert verify finds no overcounted head.

Fix: when `gone` is non-empty, set `counted = false` on both read-failure arms, since the owner is unknown; or let `release_page` do the one read and judge the page.

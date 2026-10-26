---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 93
summary: pages.rs declares mmap and madvise by hand though libc is now a dependency
closed: 2026-10-26
tags:
  - quality
---

# Feature: use libc in pages.rs

## Report

`crates/adtb-sys/src/pages.rs` declares `mmap` and `madvise` itself. Use the `libc` declarations now that `adtb-sys` depends on it.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mpwvomok.

---
created-at: 2026-10-27
status: open
kind: feature
priority: 869
summary: "heap::walk, follows, Link::heap and Heap::id take a heap number as a bare u32"
tags: [quality, engine]
---

# Feature: a newtype for the heap number a version walk checks

## Report

The heap parameter added to `heap::walk` and `follows` (`crates/adtb/src/heap/handle.rs:1316-1318`, `:1364`) is a bare `u32`, as are `Link::heap` and `Heap::id`. Callers pass `HEAP_IDS[..]` or `heap.id()` (`crates/adtb/src/exec/store.rs:248` and about ten sim test sites), where a page or slot number would also compile.

Fix: one heap-number newtype across `Heap::id`, `Link::heap` and `walk`, reusing the catalog's if it has one.

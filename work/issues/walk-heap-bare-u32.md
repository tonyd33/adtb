---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 869
summary: "heap::walk, follows, Link::heap and Heap::id take a heap number as a bare u32"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# Feature: a newtype for the heap number a version walk checks

## Report

The heap parameter added to `heap::walk` and `follows` (`crates/adtb/src/heap/handle.rs:1316-1318`, `:1364`) is a bare `u32`, as are `Link::heap` and `Heap::id`. Callers pass `HEAP_IDS[..]` or `heap.id()` (`crates/adtb/src/exec/store.rs:248` and about ten sim test sites), where a page or slot number would also compile.

Fix: one heap-number newtype across `Heap::id`, `Link::heap` and `walk`, reusing the catalog's if it has one.

## 2026-10-27: done

`heap::HeapNo(u32)` is returned by `Heap::id`, held by `Link::heap`, and taken by `walk`, `follows`, `hot_run`, `Reads::scanned` and `Reads::fetched`.

Open: `Heap::create` and `Heap::open`, the serializer, watermarks, vacuum's dead set and progress, and `Scope::Heaps` still take `u32`; their callers pass `heap.id().0`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qexec track; what stays open is in `exec-scan-follow-ups`.

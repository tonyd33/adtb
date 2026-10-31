---
created-at: 2026-10-29
status: wontfix
kind: feature
priority: 864
summary: "Heap::create and open, the serializer, watermarks, vacuum and Scope::Heaps still take u32 where the rest of the engine takes HeapNo"
closed: 2026-10-31
---

# Feature: Heap no rest

## Request

`HeapNo` is used by `Heap::id`, `Link::heap`, `walk` and the `Reads` trait. `Heap::create` and `open`, the serializer, watermarks, vacuum's dead set and progress, and `Scope::Heaps` still take `u32`. Moved from exec-scan-follow-ups.

## 2026-10-31: closed by triage

Below the filing bar: a refactor with no observable defect.

## 2026-10-31: wontfix

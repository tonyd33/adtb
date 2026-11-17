---
created-at: 2026-11-17
status: open
kind: feature
priority: 125
summary: "a write raises large-value counts before its page update can still fail, and nothing releases them on failure"
tags:
  - engine
  - class-not-atomic
---

# Feature: Retained guard

## Request

## Proposal

## Open questions

## Request

Mechanism for class-not-atomic group B, from the class count of 2026-11-17. crates/adtb/src/heap/write.rs commits `retain_apart` before `clear_written`, `reflect` and `stage.commit`, any of which can fail, leaving large-value counts raised and the values never freed.

## Proposal

`retain_apart` returns a `#[must_use]` `Retained` guard, consumed by the page commit and released on drop otherwise, keeping the order mvcc.md requires. Enforcer: the type, plus a fault sweep over large-value writes judged by verify's chunk counts.

Exit: large-retain-before-map-refusal resolved.

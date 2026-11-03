---
created-at: 2026-11-03
status: open
kind: feature
priority: 520
summary: "a pinned print or parse across a rename or add field refuses the migration where evolution.md 7.1 asks for the answer at the pinned version's names"
---

# Feature: Translate print and parse at the pinned version

## Request

## Proposal

## Open questions

`fix: name every Prim in the translation analyses` refuses a migration when a live bundle's `print` or `T.parse` would see a constructor, field, type or relation renamed, or a field added, after its pin. docs/spec/evolution.md §7.1 says a query at `v` sees the old name and rows without an added field, so the answer should be the pinned text instead. Translating needs the pinned version carried on `Prim::Print` and `Prim::Parse` through the kernel, engine and reference. The spec stays; the refusal is interim.

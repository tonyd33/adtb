---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 172
summary: "add constructor on a record type is refused, though a record is a one-constructor sum"
closed: 2026-10-25
depends-on:
  - spec-catalog-model-gaps
tags:
  - lang
---

# Feature: Add a constructor to a record type

## Report

`Source::migrate` refuses `add constructor` on a record type. The
ruling in `spec-catalog-model-gaps.md` allows it, with a recheck of
registered queries. Implement it, with tests.

## 2026-10-25: resolved

Landed as `feat(lang): give schemas catalog ids and constructor stages and fix the catalog's references`. Rulings in point `spec3`, for Tony to adopt.

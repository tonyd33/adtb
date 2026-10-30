---
created-at: 2026-10-30
status: open
kind: feature
priority: 440
summary: "sys::ctor_stats has no rows, so sys::ctor_stats.quarantined shows nothing"
---

# Feature: Ctor stats rows

## Request

The relation is declared in catalog.adt and has no rows. catalog 11.2 names its `quarantined` field as how an operator sees the state; today only `verify` rows show it.

## Proposal

## Open questions

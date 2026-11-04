---
created-at: 2026-11-03
status: open
kind: feature
priority: 640
summary: background vacuum covers user constructor relations only; catalog relations and the kept-rows heap are never vacuumed
---

# Feature: Vacuum engine relations

## Request

## Proposal

## Open questions

From the background-vacuum plan (gap 8). `catalog::*` heaps and the kept-rows heap gather dead versions that neither `vacuum r` nor the background worker reaches. State in the spec whether they are vacuumed, then schedule them by the same trigger.

---
created-at: 2026-10-26
status: open
kind: feature
priority: 799
summary: the spec does not promise that print schema orders declarations so the output replays as a first migration
tags:
  - spec
  - evolution
---

# Feature: Spec gap in print schema's declaration order

## Report

The spec does not promise that `print schema` orders declarations so the output replays as a first migration. A `redefine fn` that names a relation added later in the same printed output breaks replay, since the spec gives no ordering rule for the printed declarations beyond what is needed to parse each one on its own.

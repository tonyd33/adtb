---
created-at: 2026-11-05
status: open
kind: feature
priority: 800
summary: "Root.resolved is read only by reserve's assert since settle runs from bound"
---

# Feature: Root.resolved no longer read

## Request

## Proposal

## Open questions

## 2026-11-05

Since clog-lost, `CommitLog::settle_unresolved` settles every id from `bound`, so the root page's `resolved` field is read only by `reserve`'s assert. It is part of the on-disk root page layout and an argument of `reserve` computed by `Transactions`. Remove it with the next root page format change, or give it a reader.

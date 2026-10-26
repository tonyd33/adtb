---
created-at: 2026-10-25
status: open
kind: feature
priority: 596
summary: "storage.md does not state pager calls its consumers use"
tags: [quality]
---

# Feature: state the pager calls consumers use

## Report

Consumers call pager.position (mvcc/txn.rs), page_size, checkpoint, flush_pages, damaged and batches, none stated in docs/design/storage.md. State each as a contract or move the caller onto a stated one.

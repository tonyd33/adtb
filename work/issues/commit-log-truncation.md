---
created-at: 2026-11-03
status: open
kind: bug
priority: 480
summary: "nothing calls Transactions::truncate, so the commit log grows without bound"
---

# Truncate the commit log

## Report

From the background-vacuum plan (gap 9). `Transactions::truncate` exists and nothing calls it. Truncate below the oldest xid any live snapshot, frozen page or replica can ask about, after vacuum makes the heaps' versions all-visible; test that no lookup below the horizon is asked after truncation, over generated interleavings.

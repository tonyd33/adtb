---
created-at: 2026-10-25
status: open
kind: feature
priority: 625
summary: "the transaction layer runs only behind one latch: the heap is not Send, the commit log takes &mut Pager, and SSI lacks a few pieces"
depends-on:
  - ssi
tags:
  - engine
---

# Feature: Make the transaction layer concurrent

## Report

The SSI harness puts the pager, the commit log, the heaps and the
indexes behind one latch, because:
- `heap::Upgrade` is a `Box<dyn Fn>` without `Send + Sync`, so a `Heap`
  cannot be shared, and the harness reopens it on every operation;
- `CommitLog` and `Transactions` take `&mut Pager`.

Also missing:
- the primary logs no marker when it finds a safe snapshot
  (`docs/design/mvcc.md` §8.3), and the pager has no record for one;
- `SerializationFailure` does not name the other transactions, which
  `docs/spec/transactions.md` §6.1 puts in a system relation;
- lock promotion has per-transaction thresholds only, with no global
  memory budget;
- `mvcc/latch.rs` copies the private `storage::Lock`. Make one yielding
  lock in `adtb-sys`, and use it in both places.

Retag end to end waits for the executor (`write-path.md`).

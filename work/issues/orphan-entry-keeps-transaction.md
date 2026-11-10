---
created-at: 2026-11-09
status: open
kind: bug
priority: 240
summary: "a call refused by an index entry naming an unused slot (OrphanEntry) marks its index but leaves the transaction open, while a call that meets a damaged page ends it; decide whether a finding ends the transaction"
tags:
  - class-failure-flattened
---

# Bug: an orphan index entry leaves the transaction open

## Observed

structure-damage-policy step 6 types what made a write fail as `By { Met, Admitted, Transactions, Found }`. `Met` (a damaged page) and `Transactions` end the transaction; `Admitted` (a mark refused the call) and `Found` (an `OrphanEntry` mapped to its index's mark) leave it open. An orphan entry is evidence that the index the call read is damaged, so the reads and writes the transaction made through that index before the refusal may rest on it, as they may after a damaged page.

## Expected

A decision, stated in storage.md §8: either a finding ends the transaction as damage met does (fold `Found` into `Met`), or the reason it may stay open is stated.

## Reproduction

Read `ends_transaction` after step 6. Found in structure-damage-policy step 6.

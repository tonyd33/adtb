---
created-at: 2026-11-13
status: open
kind: bug
priority: 260
summary: "the script runner and conf compare rows as parsed values, so a change in the order of a record's printed fields passes unseen"
tags:
  - class-weak-tests
---

# Script rows compare as values and ignore field order

## Report

crates/adtb-conformance/src/engine.rs:288-335 parses each expected row with `textual::parse::<Value>` and compares Values, which ignore field order. wire.md §4.3 states that a record's fields are in the order its type declares them, and clients see the printed text and the wire order. So a script expecting `st { k: 1, b: 2 }` passes against an engine printing `st { b: 2, k: 1 }`: main sorted temp record fields this way and no script could show it (found in the review of structure-lang-identity step 6a-1, which pinned it with a Rust test instead). The order shows only in the `actual:` text of a row that differs for another reason.

Direction: compare the printed text of each row (or the value plus its field order), so a script states the order a client sees. Test: the transaction in the 6a-1 receipt with a reversed expected order fails.

Defect class: Tests asserting less than they claim.

---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 803
summary: "a split meeting the invertible wording of evolution.md 4.6 can have an inverse merge with no arm for a product, and its revert fails with a type error"
closed: 2026-10-26
---

# Feature: Split inverse rule

## Request

## Proposal

## Open questions

## Report

`split Card into Visa | Other by | Card { last4 } -> Visa {...}` meets the invertible wording of `docs/spec/evolution.md` 4.6, but its derived inverse merge has no arm for `Other`, so the revert fails with a type error. My ruling: a split is invertible only when every product has an arm; a split with a product no arm reaches is lossy, and its revert is refused naming that product. Record it in the design space and state it in 4.6.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mrtsvsxs.
Points `spec/evolution/forward-refs/later-key` and `spec/evolution/split-inverse/every-product` hold the rulings.

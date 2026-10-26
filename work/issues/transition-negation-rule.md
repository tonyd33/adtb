---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 738
summary: "a transition relation under negation makes a constraint fail on statements that touch nothing it reads, and admission accepts it"
closed: 2026-10-26
tags:
  - spec
  - engine
---

# Feature: refuse transition relations in non-monotone positions

## Report

`from a in accounts where not exists d in inserted(accounts) where key d == key a select a` passes the statement that inserts account 1 and is violated by the next statement, which touches nothing, yet neither the full check nor the delta fails it. `docs/design/maintenance.md` section 1 assumes a change touching nothing a constraint reads leaves it empty, which is false here.

My ruling: admission refuses a constraint in which `inserted` or `deleted` occurs under negation or inside an aggregate, with a refusal naming the occurrence. Record it in the design space and state it in `docs/spec/constraints.md`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as yputxooo. The rulings are points `constraint/admit/reference-index/implied` and `constraint/admit/transition-negation/monotone-only`.

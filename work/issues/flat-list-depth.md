---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 99
summary: "about 1000 items in a transact or 1000 match arms overflow a debug stack: check and normalise build terms as deep as the list is long"
closed: 2026-10-26
---

# Flat list depth

## Report

## Report

Around 1000 `let` or insert items in a `transact`, or around 1000 match arms with literal patterns, overflow a 2 MiB debug stack: check builds nested `Let` and `Block`, and the arms reach about 1000 levels in `norm::free_vars`. Make those passes iterative or the terms balanced, without counting the list against the nesting limit. `check::synth` and `binary` also cost about 14 KB per level in debug.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as woznkrwr.

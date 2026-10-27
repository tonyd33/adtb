---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 288
summary: "no test checks that every value the reference returns inhabits the type the checker gave its query, over the spec corpus and generated terms"
closed: 2026-10-26
tags:
  - reference
  - test
---

# Feature: Typing soundness test

## Report

Milestone 2 exits when typing soundness holds over the spec corpus and over type-directed generated terms. No test checks it. Write one that runs each query of the spec corpus and each term the testkit generator draws on the reference, and asserts that every returned value inhabits the type `check` gave the query, and that a write's change set inhabits its relations' row types. A failing case names the term and the value.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in vyoxxlpm. `adtb-testkit` `inhabit.rs` judges values and stored rows against checked types, keys against the database and newtype domains on stored rows. `tests/soundness.rs` judges every result and change set over the spec corpus, generated cases, fuzzed ad hoc text and the conformance suite; none is unsound. A transact result is judged against the state after the statement, and against the state before if that fails.

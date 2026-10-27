---
created-at: 2026-10-26
status: open
kind: feature
priority: 288
summary: "no test checks that every value the reference returns inhabits the type the checker gave its query, over the spec corpus and generated terms"
tags:
  - reference
  - test
---

# Feature: Typing soundness test

## Report

Milestone 2 exits when typing soundness holds over the spec corpus and over type-directed generated terms. No test checks it. Write one that runs each query of the spec corpus and each term the testkit generator draws on the reference, and asserts that every returned value inhabits the type `check` gave the query, and that a write's change set inhabits its relations' row types. A failing case names the term and the value.

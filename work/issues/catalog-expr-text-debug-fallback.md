---
created-at: 2026-11-11
status: open
kind: bug
priority: 490
summary: "catalog rows store an expression the printer does not cover as its Rust Debug text"
tags:
  - class-identity-by-spelling
---

# A catalog row holds an expression's Debug text

## Report

## 2026-11-11

`expr_text` in crates/adtb/src/catalog/rows.rs prints the expression kinds it covers as source text and falls back to `format!("{other:?}")` for the rest, so a catalog row can hold the Rust `Debug` text of an AST node. That text is unstable across refactors, unreadable to a client reading the catalog, and, if anything compares or re-parses it (not checked), identity by spelling. Found by structure-transactions step 13's search for `Debug`-text comparisons.

Direction: find which expression kinds reach `expr_text` (the index and key clauses it prints) and whether anything compares or parses its output; print every reachable kind as source text (the formatter in adtb-lang, if one covers it) and make an unreachable kind an engine invariant; a test over every clause form a schema can declare.

Defect class: Identity by spelling: a string or position stands in for the identity the type system could hold.

---
created-at: 2026-11-11
status: resolved
closed: 2026-11-15
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

## Resolved

Landed through structure-lang-identity step 10c: 10c-1 yqpztomp 0ebabda7 (migration refusal key values through `textual::print`) and 10c-2 sunvvrks 0f025d57 (2 rounds, full check 0, 3833 tests). `expr_text` is gone; an index element that is not a field path is stored and shown as its `canonical_term` text, computed once per catalog (`ElementTexts`); `change::shapes` compares that text; catalog rows a migration replaces are deleted by catalog key (`delete_keyed`); the not-admitted refusal and the exec scalar reason name the form in words. Debug sites in these printers and refusals 4 to 0. Searching test: crates/adtb/src/catalog/index_text_tests.rs.

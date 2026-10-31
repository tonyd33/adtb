---
created-at: 2026-10-26
status: in-progress
branch: tree-sitter
kind: feature
priority: 860
summary: "no tree-sitter grammar exists for adtb, so .adt and .adb files and adt code blocks get no highlighting, folding or structural editing"
tags:
  - tooling
  - lang
---

# Feature: A tree-sitter grammar for adtb

## Report

Write a tree-sitter grammar for the language of `docs/spec/syntax.md`: schema declarations, queries, writes, transactions, migration operators, bundles and ad hoc text, with the same precedence, layout and comment rules as the parser in `adtb-lang`. Ship highlight, fold, indent and locals queries, and an injection so `adt` code blocks in markdown highlight.

Done when:
- every `adt` block of `docs/spec/`, every file under `conformance/` and every file under `apps/campaign/` parses with no `ERROR` or `MISSING` node, checked by a test in the suite;
- a corpus of tree-sitter tests covers each grammar production, including the elision forms the spec examples use;
- a differential test over the front end fuzzer's programs agrees with `adtb-lang` on which texts parse;
- the grammar lives in this repo (for example `tools/tree-sitter-adtb/`), builds in the dev shell, and its generated parser is checked in.

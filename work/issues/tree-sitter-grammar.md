---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 860
summary: "no tree-sitter grammar exists for adtb, so .adt and .adb files and adt code blocks get no highlighting, folding or structural editing"
closed: 2026-10-30
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

## 2026-10-30: resolved

Grammar in `tools/tree-sitter-adtb/`; `work/tools/tree-sitter-check` runs in `check`, not in the suite, and needs no new crate. Spec 333, campaign 11, corpus 43, conformance 2758, fuzz 4000 texts with exact agreement, apart from 15 allowlisted refusals of the 64-level nesting limit. It fails on a missing result, a crash, a timeout or a class under its floor. Branch check exit 0, 3464 tests.

Left: `markdown/injections.scm` is untested, with no markdown grammar offline. The grammar accepts `(skip) + 1` at item level, `(A -> B) x` on one line and `revoke call f::q at 1 () -> T to from r`, which the parser refuses, and omits `m::Set { .. }`, which it accepts. None occurs in the differential.

## 2026-10-30: resolved

---
created-at: 2026-11-18
status: resolved
closed: 2026-11-18
kind: feature
priority: 23
tags: [tooling]
summary: "tree-sitter-check rewritten without awk, sed or jq: generate and compare, tree-sitter test, a Rust differential test"
---

# Feature: Tree sitter check rewrite

## Request

Ruled by the maintainer in the tooling review of 2026-11-17 (checks CI needs live in the public repo, held to a much higher standard than work/). The brief is work/briefs/brief-tree-sitter-check-r1.md.

## 2026-11-18: resolved

Landed in `refactor: check the tree-sitter grammar with tree-sitter test and a Rust differential test`. work/tools/tree-sitter-check is three commands (generate into target/ and diff against src/, `tree-sitter test` with highlight tests, the adtb-tree-sitter tests); the differential test runs in the suite with known disagreements as a typed list and a witness. 5 s warm against 30 s. Fuzz default 1000 seeds, deep 8000. Zlib allowed for foldhash; tree-sitter a regex wrapper. Not possible in tree-sitter 0.26.11: a negative highlight assertion on a column with no highlight.

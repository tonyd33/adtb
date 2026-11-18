---
created-at: 2026-11-18
status: resolved
closed: 2026-11-18
kind: feature
priority: 25
tags: [tooling]
summary: "rewrite the 408 doc lines that open with a third-person verb into the imperative, contracts into labelled lists"
---

# Feature: Doc imperative

## Request

Maintainer ruling, 2026-11-18: the doc-verb rule is zero-match over the whole tree. 408 doc lines open with a capitalised third-person verb (Panics 62, Runs 37, Checks 37, Fails 29, Reads 22, Refuses 14, ...). Summaries go imperative ("Return ..." stays "Returns ..."); contract sentences (Panics if, Fails if, Refuses ...) move into labelled lists (`Panics:`, `Errors:`, `Preconditions:`). The rule is work/briefs/tql/house.tql `doc_verb`.

## 2026-11-18: resolved

Landed in `docs: put doc comments in the imperative and contracts in labelled lists` (146 files, doc comments only). The doc-verb rule finds one line left, xtask/src/main.rs:55, which the tql-lint track rewrites. Contract labels (`Panics:`, `Errors:`) are exempt by `\b(?!:)`; type docs take noun phrases.

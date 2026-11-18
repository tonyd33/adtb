---
created-at: 2026-11-18
status: open
kind: feature
priority: 780
summary: adtb-tree-sitter grammar test reads conformance bodies with its own directive reader instead of adtb-conformance parse
tags:
  - quality
  - test
---

# Feature: the grammar test reads conformance files through adtb-conformance

## Report

`script_bodies` in `crates/adtb-tree-sitter/tests/grammar.rs:199-222` is a third directive reader for conformance files, beside `adtb_conformance::parse` (`crates/adtb-conformance/src/lib.rs`) and the copy in adtb-testkit's corpus that corpus-script-walk-copy covers. It treats any line whose first word is `schema`, `run`, `query` or `error` as a directive and anything else unindented as the end of a body, while `adtb_conformance::blocks` keeps a blank line inside a body only when an indented line follows and reads rows after `----`. The test `the_grammar_agrees_with_adtb_lang_on_the_conformance_scripts` (grammar.rs:239-256) also walks `conformance/` with its own `files` and keeps `.test.in` files, so the texts it compares are not exactly the texts the runner runs.

Fix: add `adtb-conformance` as a dev-dependency and read each `.test` file with `adtb_conformance::parse`, taking `Directive::body` and `line`; select files with `runner::select`. Keep the `.test.in` bodies under `conformance/gen/queries/bodies` only if they are meant to be covered, through the generated files they expand to.

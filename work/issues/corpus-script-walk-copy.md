---
created-at: 2026-10-26
status: open
kind: feature
priority: 777
summary: the differential corpus reimplements the conformance script walk and a looser directive parser
tags:
  - quality
  - test
---

# Feature: the differential corpus reads scripts through adtb-conformance's parser

## Report

`scripts` (`crates/adtb-testkit/src/corpus.rs:38-63`) and `directives` (corpus.rs:157-174) reimplement `adtb_conformance::scripts` (`crates/adtb-conformance/src/lib.rs:480`) and a looser `adtb_conformance::parse` (lib.rs:72). The two parsers can disagree on what a directive body is, and the corpus one silently skips `error` bodies and everything after `----`.

Fix: the only caller, `crates/adtb-testkit/tests/differential.rs`, already has adtb-conformance as a dev-dependency; have it pass parsed conformance scripts to `corpus::Script::from` and delete the private parser.

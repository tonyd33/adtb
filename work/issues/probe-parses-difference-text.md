---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 874
summary: "adtb-probe decides agree or differ by parsing the conformance runner's message text, so a reason containing a colon or a wording change flips the verdict"
closed: 2026-10-27
tags:
  - quality
  - test
---

# adtb-probe's verdict depends on the wording of conformance messages

## Report

`crates/adtb-probe/src/main.rs` reads each side's outcome from adtb-conformance's printed messages: `strip_prefix("got ")` at `:243-250`, `gist` splitting at `"refused with "` and the first `:` at `:271-280`, and `plan_text` splitting at `text: \"` and `\" }]` at `:283-287`. A refusal reason containing `:`, or any change to the message format, changes the verdict without an error.

Fix: have adtb-conformance return a typed outcome per directive (rows, or a refusal with its reason) and compare those.

## 2026-10-27: done

`adtb_conformance::Difference` carries `got: Option<Got>`, `Got::Rows(Vec<String>)` (printed rows in returned order) or `Got::Refused { reason, detail }`, filled by both runners for a `run` or `query` that differs; the contract is `docs/design/oracle.md` §3.1. `adtb-probe` compares sides on that: rows as a multiset, refusals by reason, a difference with no `got` agreeing with nothing; the explain plan is read by parsing the row in the textual syntax as `{ document: Text, text: Text }`. Unit tests in `crates/adtb-probe/src/main.rs`, runner tests in `tests/runner.rs` and `tests/engine_runner.rs`. Nothing stays open.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qtools track.

---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 299
summary: "prelude.md says matches uses RE2 syntax, but RE2's \\d, \\w, \\s and \\b are ASCII while both implementations use Unicode"
closed: 2026-10-25
tags:
  - spec
---

# Bug: Regex character classes

## Report

In RE2, `\d`, `\w`, `\s` and `\b` match ASCII only. Both the reference
and the executor use the `regex` crate's Unicode classes, so `\d`
matches `٣`. My ruling: the classes are Unicode, following the
database's recorded Unicode version, since `matches` works over Unicode
scalar values. State it in `prelude.md` beside the flag rules.

## 2026-10-25: resolved

Point `spec5` adopted: `\d`, `\w`, `\s` and `\b` follow the recorded Unicode version, as `docs/spec/prelude.md` now states.

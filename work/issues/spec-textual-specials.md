---
created-at: 2026-10-25
status: open
kind: feature
priority: 189
summary: "the textual forms of NaN, the infinities, time values, Bytes and Period are undefined, and syntax.md and interop.md point at each other for them"
tags: [spec]
---

# Feature: textual forms of special values

## Report

Found by the reference interpreter. No part defines the textual forms
of NaN, the infinities, time values, `Bytes` or `Period`.
`docs/spec/syntax.md` §2.5 and `docs/spec/interop.md` §1.1 point at
each other for them, and `docs/spec/prelude.md` §4 refers to forms
syntax.md does not give.

Ruling: interop §1 owns these forms. NaN is `NaN`; the infinities are
`inf` and `-inf`; times are ISO 8601; periods are `P{m}M{d}D`. `Bytes`
needs a stated form, still to be proposed. syntax.md writes them as
`T.parse("...")` calls.

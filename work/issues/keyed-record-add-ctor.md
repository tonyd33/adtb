---
created-at: 2026-10-27
status: open
kind: bug
priority: 171
summary: "add constructor on a keyed record type is refused with \"key field ... is not a header field or a field of the only constructor\""
tags:
  - lang
---

# Bug: a keyed record cannot gain a constructor

## Report

`evolution.md` section 4.2 and `model.md` section 2.2 say a record gains a constructor with an ordinary `add constructor`, and `evolve-record-add-ctor` implemented it. On a record type with a declared key the engine refuses it: "key field ... is not a header field or a field of the only constructor". The key must move into the header when the record becomes a two-constructor sum.

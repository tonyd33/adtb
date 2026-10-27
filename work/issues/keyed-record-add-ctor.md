---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 171
summary: "add constructor on a keyed record type is refused with \"key field ... is not a header field or a field of the only constructor\""
closed: 2026-10-27
tags:
  - lang
---

# Bug: a keyed record cannot gain a constructor

## Report

`evolution.md` section 4.2 and `model.md` section 2.2 say a record gains a constructor with an ordinary `add constructor`, and `evolve-record-add-ctor` implemented it. On a record type with a declared key the engine refuses it: "key field ... is not a header field or a field of the only constructor". The key must move into the header when the record becomes a two-constructor sum.

## 2026-10-27: done

When a record with a declared key gains a constructor, the key fields of its relations move to the header, keeping their ids; with no other fields the record's constructor is nullary. The revert sets the record text back, header field ids returning to the record's constructor. The old evolve test that pinned the refusal now states the new behaviour; `evolution.md` 4.2 says the new constructor has the key fields.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qlaws track.

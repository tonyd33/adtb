---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 545
summary: "the heap's row type, a contract its consumers rely on, is stated only in a non-binding note"
branch: adtb-verify
depends-on:
  - slotted-heap
tags:
  - design
  - engine
---

# Feature: State the heap's row type as a contract

## Report

`docs/design/mvcc.md` §12.2 says a row is `Vec<adtb::key::Value>`,
described by the heap's `FieldType` and `CtorType`, with a sum's `ctor`
the declaration-order index, and that a heap always returns rows in its
newest format. The executor and the write path will rely on this, but
§12 is notes for the implementation. Move the contract into a binding
section, and keep the byte layout in §12.2.

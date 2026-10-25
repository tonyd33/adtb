---
title: "What deduplication of write calls promises"
pick: one
options:
  promise:
    title: "A resend after commit returns the original reply and never reapplies"
  recorded:
    title: "State how the engine records ids and replies"
---
Raised by the tier review of docs/spec/. transactions §9.2 said the
engine records the id and reply in the same commit, a mechanism.

## spec/txn/resend/promise

A resend of a committed request returns the original reply, rows
included, and never applies the write again.

## spec/txn/resend/recorded

The spec states the record kept in the commit. Names a representation
the caller cannot observe.

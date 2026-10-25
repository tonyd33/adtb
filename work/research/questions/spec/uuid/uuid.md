---
title: "How uuid keys are represented"
pick: one
options:
  uuid-newtype:
    title: "A prelude Uuid newtype over 16 bytes with uuid() and text()"
  bytes:
    title: "Raw Bytes"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/uuid/uuid-newtype

A prelude Uuid newtype over 16 bytes with uuid() and text(). Keys need a readable textual form.

## spec/uuid/bytes

Raw Bytes. No textual form.

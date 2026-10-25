---
created-at: 2026-10-25
status: open
kind: feature
priority: 712
summary: "batches carry a second value type, Datum, because key::Value has no list; offsets are i32 with no large variant"
depends-on:
  - batches
tags:
  - engine
---

# Feature: One result value type, and large offsets

## Report

`adtb::batch::Datum` is `adtb::key::Value` plus lists, needed for
nested results. When the executor defines its result value type, make
it the one type and drop `Datum`.

Text, binary and list offsets are `i32`, so one column holds less than
2 GiB, and pushing past that panics. Morsels never come near it, but a
builder must refuse instead of panicking, or use 64-bit offsets.

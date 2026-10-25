---
title: "Where per-version flags live"
pick: one
options:
  slot:
    title: "In the slot; the version header stays 24 bytes"
  header:
    title: "In a 32-byte version header"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/header-24/slot

In the slot; the version header stays 24 bytes. Keeps the ruled 24-byte header with 64-bit tids.

## spec/header-24/header

In a 32-byte version header. 8 more bytes per row.

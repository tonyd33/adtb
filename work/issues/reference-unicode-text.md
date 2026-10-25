---
created-at: 2026-10-25
status: open
kind: feature
priority: 250
summary: "the reference interpreter returns not yet for the Unicode text functions, and does not re-normalise built text to NFC"
tags: [reference]
---

# Feature: Unicode text in the reference interpreter

## Report

In `adtb-reference`, `grapheme_len`, `fold`, `matches`, `collate`, the
zone functions and `T.parse` return "not yet". Text built by `++`,
`lower` and `upper` is not re-normalised to NFC.

These need Unicode tables. `unicode-normalization` is approved in
`deny.toml`; grapheme segmentation and case folding need a decision on
a crate.

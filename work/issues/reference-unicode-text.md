---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 250
summary: "the reference interpreter returns not yet for the Unicode text functions, and does not re-normalise built text to NFC"
closed: 2026-10-25
tags:
  - reference
---

# Feature: Unicode text in the reference interpreter

## Report

In `adtb-reference`, `grapheme_len`, `fold`, `matches`, `collate`, the
zone functions and `T.parse` return "not yet". Text built by `++`,
`lower` and `upper` is not re-normalised to NFC.

These need Unicode tables. `unicode-normalization` is approved in
`deny.toml`; grapheme segmentation and case folding need a decision on
a crate.

## 2026-10-25: crates

Tony chose the ICU4X stack: `icu_segmenter`, `icu_casemap` and `icu_collator`, with `regex` for `matches` and `jiff` with its bundled time zone database for the zone functions.

## 2026-10-25: resolved

Landed as `feat(reference): evaluate Unicode text and zone functions and read and print the textual syntax`. Spec gaps found are in `spec-text-gaps.md`.

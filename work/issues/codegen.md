---
created-at: 2026-10-25
status: open
kind: feature
priority: 840
summary: no generator emits typed Rust calls from a bundle file
depends-on:
  - bundles
  - embedded-open
tags:
  - engine
  - lang
---

# Feature: Rust code generation from a bundle file

## Report

Implement `docs/design/kernel.md` §8: Rust code generation from a
bundle file, typed calls over the in-process API. Settle `kernel.md:398`,
the API by which `adtb-lang` gives a generator ids and ordinals.

Prerequisites: `bundles.md`, `embedded-open.md`.

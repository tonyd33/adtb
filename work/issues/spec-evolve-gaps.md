---
created-at: 2026-10-26
status: open
kind: feature
priority: 530
summary: "the .adtx package format and revoke of a privilege never granted are unspecified"
tags:
  - spec
---

# Feature: evolution spec gaps

## Report

- `interop.md` section 9.6 says only that a package is "a module of definitions with the Wasm code they use, packaged as one file". The file's layout, how its declarations are read, and how its code is named are unstated. The checker assumes declaration text.
- What `revoke` of a privilege that was never granted does is unstated. The checker accepts it.

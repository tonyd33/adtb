---
created-at: 2026-10-27
status: open
kind: feature
priority: 843
summary: "the campaign app runs its bundle bodies as ad hoc text instead of calling generated code"
tags:
  - app
---

# Feature: port the campaign app to generated calls

## Report

`adtb-codegen` generates and compiles about 4400 lines for `apps/campaign`. Its tests still run bundle bodies as ad hoc text, because `campaign-1.0.adb` pins version 1 and calls of an earlier-pinned bundle are refused `Untranslatable` until `version-translation` lands. Write a bundle pinned at the app's current version with its hash, add a `build.rs` with `adtb-codegen` as a build dependency, and rewrite the 54 `app.call`, `ok` and `one` sites in `tests/engine.rs` to typed calls.

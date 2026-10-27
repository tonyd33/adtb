---
created-at: 2026-10-26
status: open
kind: feature
priority: 824
summary: the ad hoc checker encodes no bundles both as None and as an empty Bundles
tags:
  - quality
  - lang
---

# Feature: one encoding of no bundles in the ad hoc checker

## Report

`Bundles::params` (`crates/adtb-lang/src/check/adhoc.rs:22-40`) is an associated function taking `Option<&Bundles>`, whose doc says "`None` knowing no bundle" though it returns an error then. `Checker.bundles` (`crates/adtb-lang/src/check/mod.rs:167`) is an `Option`, yet `check_adhoc_recording` always sets `Some` and `check_adhoc` passes `&Bundles::default()`. `None` only means a checker that is not ad hoc, which never reaches explain. "No bundles" has two encodings.

Fix: make `params` a method on `&Bundles`, and give `Checker` a `&Bundles` that defaults to an empty one.

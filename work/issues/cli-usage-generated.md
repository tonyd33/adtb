---
created-at: 2026-10-30
status: open
kind: feature
priority: 460
summary: "three binaries hand-write their usage line with override_usage, which can drift from their flags"
---

# Feature: Cli usage generated

## Request

adtb, adtb-probe and adtb-codegen set `override_usage` to keep the usage line of the old parsers. Tony asked why, 2026-10-30. Drop it and let clap generate the line; update docs/manual/reference/shell.md and the tests in crates/adtb-shell/tests/shell.rs that match the old text.

## Proposal

## Open questions

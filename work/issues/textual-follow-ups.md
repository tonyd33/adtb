---
created-at: 2026-10-25
status: open
kind: feature
priority: 593
summary: "the shared printer panics on a mistyped value, Textual for Val panics on a list, and copies of parse_uuid and self_key remain"
tags: [quality]
---

# Feature: textual syntax follow-ups

## Report

From the textual syntax track:
- `adtb_lang::textual::print` and `to_text` panic when a value does not match its `Form`; before the move the engine returned `Error::Internal` there. Make them return a `Result` the engine maps to `Internal`.
- `Textual for Val` panics in `elems` on a `Val::List`; the shell prints ordered results.
- `crates/adtb-reference/src/prim.rs` has its own `parse_uuid`; use `adtb_lang::textual::parse_uuid`.
- `self_key` and the key value type logic are copied in `crates/adtb/src/write/schema.rs` and `catalog/layout.rs`; use the `adtb-lang` versions.
- The round-trip tests in `crates/adtb-reference/tests/textual.rs` duplicate the new `adtb-lang` ones; delete them once `adtb-testkit` lands, and move the random value drawer duplicated in the `adtb-lang` and `adtb-sim` tests into `adtb-testkit`.

---
created-at: 2026-10-25
status: open
kind: feature
priority: 176
summary: "temp relations are never declared, so later items cannot name them and 2 spec blocks fail"
tags: [lang]
---

# Feature: temp relations

## Report

A `temp` declaration in ad hoc text declares nothing: later items that
name the relation fail to resolve. 2 blocks fail `spec_examples_check`.

```
cargo test -p adtb-lang --test spec_corpus spec_examples_check
```

Actual:

```
interop.md:415:
  interop.md:418: no `identity_stage` in scope
interop.md:458:
  interop.md:464: no `identity_dupes` in scope
  interop.md:469: `identity_stage` is a parameter of this text, but the call supplies no value for it
```

Expected: both blocks check.

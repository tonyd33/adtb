---
created-at: 2026-10-25
status: open
kind: feature
priority: 177
summary: "user-defined aggregates are refused unchecked, so interop.md:1053 fails"
tags: [lang]
---

# Feature: check user-defined aggregates

## Report

The checker refuses every user-defined aggregate.

```
cargo test -p adtb-lang --test spec_corpus spec_examples_check
```

Actual:

```
interop.md:1053:
  interop.md:1064: user-defined aggregates are not checked yet
```

Expected: the block checks.

---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 170
summary: "migration operators are not type checked, so 10 spec blocks fail"
branch: adtb-lang3
tags:
  - lang
---

# Feature: check migration operators

## Report

The checker refuses every schema modification operator with "migration
operators are not checked yet". 10 blocks fail `spec_examples_check`
for it: `constraints.md:60`, `derived.md:327`, `derived.md:340`,
`evolution.md:664`, `evolution.md:716`, `evolution.md:749`,
`evolution.md:872`, `syntax.md:1230`, `syntax.md:1261` and
`writes.md:590` (`spec-example-writes-590.md`).

```
cargo test -p adtb-lang --test spec_corpus spec_examples_check
```

Actual, among others:

```
evolution.md:749:
  evolution.md:753: migration operators are not checked yet
  evolution.md:756: migration operators are not checked yet
  evolution.md:761: migration operators are not checked yet
```

Expected: every block checks.

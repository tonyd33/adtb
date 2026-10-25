---
created-at: 2026-10-25
status: open
kind: feature
priority: 160
summary: "the checker has no catalog:: or sys:: modules and refuses engine declarations, so 30 spec blocks fail"
tags: [lang]
---

# Feature: engine catalog modules

## Report

The checker declares no `catalog::` or `sys::` module, and it refuses
the forms only the engine's own declarations use: qualified relation
names and `volatile`. 30 blocks fail `spec_examples_check`: 29 in
`docs/spec/catalog.md` and `docs/spec/security.md:578`.

```
cargo test -p adtb-lang --test spec_corpus spec_examples_check
```

Actual, among others:

```
catalog.md:1233:
  catalog.md:1234: no `calls` in scope
  catalog.md:1235: no module `catalog`
  catalog.md:1236: no type `WaitReason`
  catalog.md:1237: `volatile` appears only in the engine's own declarations
catalog.md:1734:
  catalog.md:1755: a relation name is qualified only in the engine's own declarations
security.md:578:
  security.md:579: no module `catalog`
```

Expected: every block checks against the engine's declarations of
`catalog.md`.

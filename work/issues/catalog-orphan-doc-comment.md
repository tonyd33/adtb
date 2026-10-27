---
created-at: 2026-10-26
status: open
kind: bug
priority: 767
summary: "oversized was inserted under ctor_relations' doc comment, so it carries two docs and ctor_relations none"
tags:
  - quality
  - engine
---

# oversized carries ctor_relations' doc comment, and ctor_relations has none

## Report

`crates/adtb/src/catalog/mod.rs:270-273`: the new `fn oversized` was inserted between `ctor_relations`'s doc comment and its body, so `oversized` has two stacked "Returns" docs and `ctor_relations` (mod.rs:295) has none.

Fix: move the first two doc lines back above `ctor_relations`.

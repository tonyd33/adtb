---
created-at: 2026-11-13
status: open
kind: bug
priority: 800
summary: "three migration operators call held_id on a Source id lookup, so a miss panics naming a built schema's invariant instead of name_ids'"
tags:
  - lang
  - quality
  - class-failure-flattened
---

# `held_id` on a Source lookup names the wrong invariant

## Report

`held_id` (crates/adtb-lang/src/ids/mod.rs:206) panics with "every object of a built schema outside the temp module has a catalog id". Three operator sites pass it `self.src.ctor_key_id(..)`, a lookup in `Source::ids` right after `self.src.name_ids()`, not in a built schema:

- crates/adtb-lang/src/evolve/cx.rs:461
- crates/adtb-lang/src/evolve/ctor/mod.rs:285
- crates/adtb-lang/src/evolve/ctor/mod.rs:1200

What these sites rely on is that `name_ids` gave every parsed constructor an id. `name_ids` returns without giving any id when the declarations do not parse (crates/adtb-lang/src/evolve/ids.rs:45). A miss there would report an invariant of built schemas that does not apply.

Found by reading; not reproduced.

Fix direction: give these lookups their own expect naming `name_ids`' postcondition, or have `name_ids` return the ids it gave so the operator reads its constructor's id from that result.

Defect class: class-failure-flattened

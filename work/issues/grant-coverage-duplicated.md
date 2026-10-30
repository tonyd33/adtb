---
created-at: 2026-10-29
status: open
kind: bug
priority: 896
summary: "permission/grants.rs targets and evolve/carry.rs covered_ctors each resolve a grant target to the constructors it covers, so carry and the registration check can disagree"
tags:
  - quality
  - lang
---

# Grant target coverage is computed twice

## Report

`crates/adtb-lang/src/permission/grants.rs:62` `targets` (added by checked-registration) and `crates/adtb-lang/src/evolve/carry.rs:905` `covered_ctors` each resolve a `GrantTarget` to the constructors it covers in a schema. Each has its own `Whole`, `Ctors` and `Set` arms. A named set is matched by iterating the relation's sets and looking each id up in one, and by finding the set id and then the set by name in the other. Carry decides what a grant still covers after a migration, and the registration check decides what the same grant allows. If they disagree, carry keeps a grant that the check reads differently. `permission/grants.rs:86` `relation`, `evolve/grant_text.rs:118` `target` and `carry.rs:906` also each repeat the catalog-id-to-`RelId` reverse lookup that `crates/adtb/src/catalog/ids.rs:72` provides in the engine.

Fix: one `GrantTarget::ctors(schema)` in `evolve`, plus a reverse id lookup on `schema::Ids`, used by carry, the permission resolver and grant text.

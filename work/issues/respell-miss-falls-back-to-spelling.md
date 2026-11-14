---
created-at: 2026-11-13
status: open
kind: bug
priority: 600
summary: "an id the revert's id map misses keeps its old value, and respelling then writes the name as first spelled, so a recreate miss resolves by spelling instead of being refused"
tags:
  - lang
  - quality
  - class-identity-by-spelling
---

# A missed re-created id is respelled by its old spelling

## Report

`again_id` (crates/adtb-lang/src/evolve/revert/by_id.rs:165) returns the original id when the revert's `Recreated` map has no entry for it. `Spelled` respelling (crates/adtb-lang/src/evolve/revert/spelled.rs:160) refuses a name whose ids a later migration dropped (`late`). It writes the current name when the catalog holds one of its ids (`now`). When neither holds an id, it writes the name as first spelled (`(None, _) => out.push(written)`, spelled.rs:181), and the restored text then resolves it by spelling.

`recreate` is meant to map every object the reverted operators removed. When it misses one, that object's old id is in neither `now` nor `late`, and the restored text names whatever now has that spelling. The fallback is also the path for a name of an object the same step re-creates, such as a split inverse's arm naming the constructor it brings back. A miss therefore cannot be told apart from a legitimate case, and nothing enforces the "a miss is refused later" rule recorded for step 5c-1. For `Rename`, `DropSet` and `Redeclare`, a miss is refused, but with the reason that a later migration dropped the object.

Found by reading; not reproduced. No input producing a miss today is known.

Fix direction: pass respelling the ids the current step re-creates. Refuse a name whose ids are in none of `now`, `late` or that set, with an internal-invariant reason, and give `again_id` the same refusal instead of the identity fallback.

Defect class: class-identity-by-spelling

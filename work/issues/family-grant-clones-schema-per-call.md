---
created-at: 2026-10-31
status: open
kind: bug
priority: 725
summary: "each call a family grant covers clones a schema and recomputes signature ids twice, once at submit and once leaving the queue"
tags:
  - quality
  - engine
---

# Family grant clones schema per call

## Report

`RoleBinding::submit` and `recheck` both run `admit_call`, each with a new `Calls` (crates/adtb/src/api/server/binding.rs:75, `holds_call`). A family row reaches `family_covers` (crates/adtb-lang/src/evolve/grant.rs) and so `Calls::family` (crates/adtb/src/api/server/grant.rs:190), whose memo lives only as long as that `Calls`. Each miss parses the target, runs `source_at`, then `Source::schema`, which clones the whole schema (crates/adtb-lang/src/evolve/source.rs:80), then `signature_ids`. `Calls::query` recomputes the bundle's `Ids` as well.

So a role's call through a family grant does this work at least twice per call, for each family row of the role in front of the one that covers it, growing with the schema's size. A bundle grant does none of it. `submit` runs this on the caller's thread and `recheck` on a worker.

Found by reading; not measured. Measure with the benchmarks before changing.

Direction: a family target's text names a fixed version, so its ids never change: hold them on the engine by target. Hold each served callable's `Ids` on its `Entry`.
